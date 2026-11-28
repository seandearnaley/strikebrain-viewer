//! One local SQLite database per source and encoder revision. A file's
//! vectors and identifying metadata commit together, including on pause.
use std::fs::{self, File, Metadata};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context as _, Result, ensure};
use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

use super::encoder::WIDTH;
use super::pack::REVISION;
use crate::stage::load::open_regular;

pub(super) struct Index {
    db: Connection,
    root: PathBuf,
    _lock: File,
}

fn index_dir(base: &Path) -> PathBuf {
    base.join("indexes").join(format!("{REVISION}-{WIDTH}-v2"))
}

pub(crate) fn index_path(base: &Path, root: &Path) -> PathBuf {
    let key = Sha256::digest(root.as_os_str().as_encoded_bytes());
    index_dir(base).join(format!("{}.sqlite", hex::encode(key)))
}

#[derive(Debug)]
pub(crate) struct StoredIndex {
    pub path: PathBuf,
    pub label: String,
    pub bytes: u64,
}

pub(crate) fn catalog(base: &Path) -> Result<Vec<StoredIndex>> {
    let dir = index_dir(base);
    if !dir.try_exists()? {
        return Ok(Vec::new());
    }
    let mut list = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "sqlite") {
            continue;
        }
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let label = db.query_row("SELECT name FROM source LIMIT 1", [], |row| row.get(0))?;
        list.push(StoredIndex {
            bytes: path.metadata()?.len(),
            path,
            label,
        });
    }
    list.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(list)
}

pub(crate) fn delete_index(path: &Path) -> Result<()> {
    let lock = File::create(path.with_extension("lock"))?;
    lock.try_lock()
        .context("Pause indexing this folder before deleting its index")?;
    Ok(fs::remove_file(path)?)
}

impl Index {
    pub(super) fn open(base: &Path, root: &Path) -> Result<Self> {
        fs::create_dir_all(index_dir(base))?;
        let path = index_path(base, root);
        let lock = File::create(path.with_extension("lock"))?;
        lock.lock_shared()?;
        let db = Connection::open(path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS files(path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, vectors BLOB NOT NULL) WITHOUT ROWID;
             CREATE TEMP TABLE seen(path TEXT PRIMARY KEY) WITHOUT ROWID;",
        )?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS source(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        )?;
        db.execute(
            "INSERT OR IGNORE INTO source VALUES(1,?)",
            [root.to_string_lossy().as_ref()],
        )?;
        Ok(Self {
            db,
            root: root.to_owned(),
            _lock: lock,
        })
    }

    pub(super) fn cached(&self) -> Result<Vec<super::Entry>> {
        let mut stmt = self
            .db
            .prepare("SELECT path, vectors FROM files ORDER BY path")?;
        let mut rows = stmt.query([])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let relative = PathBuf::from(row.get::<_, String>(0)?);
            ensure!(
                !relative.as_os_str().is_empty()
                    && relative
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_))),
                "Invalid cached search path"
            );
            entries.push(super::Entry {
                path: self.root.join(relative),
                vectors: decode(&row.get::<_, Vec<u8>>(1)?)?,
            });
        }
        Ok(entries)
    }

    /// Reconcile only after a complete, readable inventory.
    pub(super) fn finish_inventory(&self) -> Result<()> {
        self.db.execute(
            "DELETE FROM files WHERE path NOT IN (SELECT path FROM seen)",
            [],
        )?;
        Ok(())
    }

    pub(super) fn vectors(
        &self,
        path: &Path,
        encode: impl FnOnce(File) -> Result<Vec<f32>>,
    ) -> Result<Vec<f32>> {
        let key = path
            .strip_prefix(&self.root)?
            .to_str()
            .context("Invalid search path")?;
        let meta = fs::symlink_metadata(path)?;
        ensure!(meta.is_file(), "File is no longer a regular file");
        let stamp = stamp(&meta)?;
        let size = i64::try_from(meta.len())?;
        let cache: Option<Vec<u8>> = self
            .db
            .prepare_cached("SELECT vectors FROM files WHERE path=? AND size=? AND mtime=?")?
            .query_row(params![key, size, stamp], |row| row.get(0))
            .optional()?;
        if let Some(bytes) = cache {
            let vectors = decode(&bytes)?;
            self.db
                .prepare_cached("INSERT OR IGNORE INTO seen VALUES(?)")?
                .execute([key])?;
            return Ok(vectors);
        }
        let (file, opened) = open_regular(path)?.context("File is no longer readable")?;
        let vectors = encode(file)?;
        let after = fs::symlink_metadata(path)?;
        ensure!(
            after.is_file()
                && opened.len() == meta.len()
                && after.len() == meta.len()
                && self::stamp(&after)? == stamp
                && self::stamp(&opened)? == stamp,
            "File changed during indexing"
        );
        let bytes: Vec<u8> = vectors
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.db
            .prepare_cached("INSERT OR REPLACE INTO files VALUES(?,?,?,?)")?
            .execute(params![key, size, stamp, bytes])?;
        self.db
            .prepare_cached("INSERT OR IGNORE INTO seen VALUES(?)")?
            .execute([key])?;
        Ok(vectors)
    }
}

fn decode(bytes: &[u8]) -> Result<Vec<f32>> {
    ensure!(
        !bytes.is_empty() && bytes.len().is_multiple_of(WIDTH * 4),
        "Damaged search index"
    );
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

fn stamp(meta: &Metadata) -> Result<i64> {
    Ok(i64::try_from(
        meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_source_keeps_only_matches_from_the_last_complete_inventory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("share");
        fs::create_dir(&root).unwrap();
        let store = tmp.path().join("store");
        let first = root.join("one.txt");
        let second = root.join("two.txt");
        for path in [&first, &second] {
            fs::write(path, "sea").unwrap();
        }
        let index = Index::open(&store, &root).unwrap();
        for path in [&first, &second] {
            index.vectors(path, |_| Ok(vec![0.25; WIDTH])).unwrap();
        }
        index.finish_inventory().unwrap();
        drop(index);
        fs::remove_file(&first).unwrap();
        let index = Index::open(&store, &root).unwrap();
        index.vectors(&second, |_| panic!("cached")).unwrap();
        index.finish_inventory().unwrap();
        drop(index);
        fs::remove_dir_all(&root).unwrap();
        let entries = Index::open(&store, &root).unwrap().cached().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, second);
        assert_eq!(entries[0].vectors, vec![0.25; WIDTH]);
    }
    #[test]
    fn caches_complete_vectors_and_replaces_changed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("art");
        fs::create_dir(&root).unwrap();
        let store = tmp.path().join("data");
        let path = root.join("file.md");
        fs::write(&path, "sea").unwrap();
        let index = Index::open(&store, &root).unwrap();
        let first = vec![0.25; WIDTH];
        assert_eq!(index.vectors(&path, |_| Ok(first.clone())).unwrap(), first);
        drop(index);
        let index = Index::open(&store, &root).unwrap();
        assert_eq!(
            index
                .vectors(&path, |_| panic!("unchanged source was encoded"))
                .unwrap(),
            first
        );
        fs::write(&path, "mountains").unwrap();
        assert_eq!(
            index
                .vectors(&path, |_| anyhow::bail!("decode failed"))
                .unwrap_err()
                .to_string(),
            "decode failed"
        );
        assert_eq!(
            index.vectors(&path, |_| Ok(vec![0.75; WIDTH])).unwrap(),
            vec![0.75; WIDTH]
        );
        let list = catalog(&store).unwrap();
        assert_eq!(list.len(), 1);
        delete_index(&list[0].path).unwrap_err();
        drop(index);
        delete_index(&list[0].path).unwrap();
        assert!(path.exists());
        assert!(catalog(&store).unwrap().is_empty());
    }
}
