//! Validated, local directory listings for macOS SMB mounts. A fresh open of
//! *each* directory checks read access, identity and both change timestamps.
//! Names and types alone are reusable. Ignore files and file sizes stay live.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Stat, fstat, open, statat, statfs};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{ALWAYS_SKIPPED, CacheMode, ScanError, ScanOptions, rules, walk_threads};
use crate::tree::RawTree;
use crate::{Kind, Tree};

pub(super) fn is_smb(root: &Path) -> Result<bool, ScanError> {
    let mount = statfs(root).map_err(|err| ScanError::Root(err.into()))?;
    Ok(mount
        .f_fstypename
        .map(|c| c.to_ne_bytes()[0])
        .starts_with(b"smbfs\0"))
}

pub(super) fn scan(
    root: &Path,
    options: ScanOptions,
    cancel: &AtomicBool,
    progress: Option<&AtomicUsize>,
    cache: &Path,
    mode: CacheMode,
) -> Result<Tree, ScanError> {
    let (root, file) = super::resolve_root(root)?;
    if let Some(size) = file {
        return Ok(Tree::file(&root, size));
    }
    // A redirected state store may sit in the folder being watched. Writing
    // its index there would change the listing and trigger another scan.
    if cache.starts_with(&root) {
        return super::scan_observed(&root, options, cancel, progress);
    }
    let manifest = cache.join(manifest_name(&root));
    indexed(&root, options, cancel, progress, &manifest, mode).map(|(tree, _)| tree)
}

fn manifest_name(root: &Path) -> String {
    let key = Sha256::digest(root.as_os_str().as_bytes());
    let mut name = String::with_capacity(69);
    for byte in key {
        write!(name, "{byte:02x}").expect("writing to a String is infallible");
    }
    name.push_str(".json");
    name
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    device: i32,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl From<Stat> for Stamp {
    fn from(s: Stat) -> Self {
        Self {
            device: s.st_dev,
            inode: s.st_ino,
            modified: (s.st_mtime, s.st_mtime_nsec),
            changed: (s.st_ctime, s.st_ctime_nsec),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Listing {
    stamp: Option<Stamp>,
    entries: Vec<(Name, Kind)>,
}

// JSON strings keep the common case compact. Preserve native bytes when
// the name cannot be represented as text, including for ignore matching.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Name {
    Text(String),
    Native(OsString),
}

impl Name {
    fn from_os(name: &OsStr) -> Self {
        name.to_str().map_or_else(
            || Self::Native(name.to_owned()),
            |s| Self::Text(s.to_owned()),
        )
    }

    fn as_os(&self) -> &OsStr {
        match self {
            Self::Text(text) => text.as_ref(),
            Self::Native(name) => name,
        }
    }
}

type Manifest = Vec<(OsString, Listing)>;

fn load(path: &Path) -> HashMap<OsString, Listing> {
    let result = fs::read(path).and_then(|bytes| Ok(serde_json::from_slice::<Manifest>(&bytes)?));
    match result {
        Ok(dirs) => dirs.into_iter().collect(),
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                log::warn!("Reading SMB listing cache {}: {err}", path.display());
            }
            HashMap::new()
        }
    }
}

fn save(path: &Path, dirs: &Manifest, cancel: &AtomicBool) -> io::Result<()> {
    let parent = path
        .parent()
        .expect("the manifest is inside the cache directory");
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut writer = BufWriter::new(temp.as_file_mut());
    serde_json::to_writer(&mut writer, dirs)?;
    writer.flush()?;
    drop(writer);
    if !cancel.load(Ordering::Relaxed) {
        temp.persist(path)?;
    }
    Ok(())
}

fn listing(
    path: &Path,
    cached: Option<Listing>,
    cancel: &AtomicBool,
    progress: Option<&AtomicUsize>,
) -> io::Result<(Listing, bool)> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW_ANY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stamp = Stamp::from(fstat(&fd)?);
    if let Some(old) = cached
        && old.stamp.as_ref() == Some(&stamp)
        && old.entries.iter().all(|(name, kind)| {
            let name = name.as_os();
            Path::new(name).file_name() == Some(name)
                && *kind != Kind::Unreadable
                && !name.as_bytes().contains(&0)
                && !ALWAYS_SKIPPED.iter().any(|&skip| name == skip)
        })
    {
        if let Some(counter) = progress {
            counter.fetch_add(old.entries.len(), Ordering::Relaxed);
        }
        return Ok((old, false));
    }
    let mut dir = Dir::new(fd)?;
    let mut entries = Vec::new();
    let mut complete = true;
    while let Some(entry) = dir.next() {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let entry = entry?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." || ALWAYS_SKIPPED.iter().any(|&skip| name == skip) {
            continue;
        }
        let mut ty = entry.file_type();
        if ty == FileType::Unknown {
            let Ok(stat) = statat(dir.fd()?, name, AtFlags::SYMLINK_NOFOLLOW) else {
                complete = false;
                continue;
            };
            ty = FileType::from_raw_mode(stat.st_mode);
        }
        let kind = match ty {
            FileType::Directory => Kind::Dir,
            FileType::RegularFile => Kind::File,
            FileType::Symlink => Kind::Symlink,
            _ => continue,
        };
        entries.push((Name::from_os(name), kind));
        if entries.len().is_multiple_of(super::PROGRESS_BATCH)
            && let Some(counter) = progress
        {
            counter.fetch_add(super::PROGRESS_BATCH, Ordering::Relaxed);
        }
    }
    if let Some(counter) = progress {
        counter.fetch_add(entries.len() % super::PROGRESS_BATCH, Ordering::Relaxed);
    }
    let stable = complete && stamp == Stamp::from(dir.stat()?);
    Ok((
        Listing {
            stamp: stable.then_some(stamp),
            entries,
        },
        true,
    ))
}

struct Job {
    id: u32,
    relative: PathBuf,
    cached: Option<Listing>,
}

type ResultRow = (Job, io::Result<(Listing, bool)>);

/// One coordinator owns the tree and the live ignore matcher. Workers return
/// one listing at a time, so a large sibling never holds up descent elsewhere.
pub(super) fn indexed(
    root: &Path,
    options: ScanOptions,
    cancel: &AtomicBool,
    progress: Option<&AtomicUsize>,
    manifest: &Path,
    mode: CacheMode,
) -> Result<(Tree, usize), ScanError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    let previous = match mode {
        CacheMode::Validate => load(manifest),
        CacheMode::Rebuild => HashMap::new(),
    };
    let original_count = previous.len();
    let (raw, current, reads) = {
        let (send, jobs) = mpsc::channel::<Job>();
        let jobs = Mutex::new(jobs);
        let (done, results) = mpsc::channel();
        // The receiver lives across all worker tasks. Dropping the last job
        // sender below wakes every worker, including on cancellation/error.
        std::thread::scope(|workers| {
            for _ in 0..walk_threads() {
                let done = done.clone();
                let jobs = &jobs;
                workers.spawn(move || {
                    loop {
                        let job = jobs.lock().expect("job queue poisoned").recv();
                        let Ok(mut job) = job else { break };
                        let result = listing(
                            &root.join(&job.relative),
                            job.cached.take(),
                            cancel,
                            progress,
                        );
                        if done.send((job, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(done);
            let result = collect(root, options, cancel, previous, &send, &results);
            drop(send);
            drop(results);
            result
        })
    }?;
    let tree = raw.build(root.to_owned())?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    if (reads > 0 || current.len() != original_count)
        && let Err(err) = save(manifest, &current, cancel)
    {
        log::warn!("Writing SMB listing cache {}: {err}", manifest.display());
    }
    Ok((tree, reads))
}

fn collect(
    root: &Path,
    options: ScanOptions,
    cancel: &AtomicBool,
    mut previous: HashMap<OsString, Listing>,
    send: &mpsc::Sender<Job>,
    results: &mpsc::Receiver<ResultRow>,
) -> Result<(RawTree, Manifest, usize), ScanError> {
    let mut current = Vec::with_capacity(previous.len());
    let mut reads = 0;
    let mut matcher = rules(root, options)
        .build_matchers()
        .pop()
        .expect("one matcher for one root");
    let root_name = root
        .file_name()
        .unwrap_or(root.as_os_str())
        .to_string_lossy();
    let mut raw = RawTree::new(&root_name, false)?;
    let mut enqueue = |id, relative: PathBuf| {
        let cached = previous.remove(relative.as_os_str());
        send.send(Job {
            id,
            relative,
            cached,
        })
        .expect("the worker queue is alive through collection");
    };
    enqueue(0, PathBuf::new());
    let mut pending = 1usize;
    while pending > 0 {
        let (job, result) = results
            .recv()
            .map_err(|err| ScanError::Root(io::Error::other(err)))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }
        pending -= 1;
        let (listing, read) = match result {
            Ok(found) => found,
            Err(err) if job.id == 0 => return Err(ScanError::Root(err)),
            Err(_) => {
                raw.set_kind(job.id, Kind::Unreadable);
                continue;
            }
        };
        reads += usize::from(read);
        for (i, (name, kind)) in listing.entries.iter().enumerate() {
            if i.is_multiple_of(super::PROGRESS_BATCH) && cancel.load(Ordering::Relaxed) {
                return Err(ScanError::Cancelled);
            }
            let name = name.as_os();
            let relative = job.relative.join(name);
            if matcher.matched(&relative, *kind == Kind::Dir).is_ignore() {
                continue;
            }
            let id = raw.push(job.id, &name.to_string_lossy(), *kind)?;
            if *kind == Kind::Dir {
                enqueue(id, relative);
                pending += 1;
            }
        }
        if listing.stamp.is_some() {
            current.push((job.relative.into_os_string(), listing));
        }
    }
    Ok((raw, current, reads))
}
