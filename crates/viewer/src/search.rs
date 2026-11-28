//! On-demand local content search. The worker owns inference and storage.
//! GPUI receives bounded result lists and never opens source media here.
mod encoder;
mod index;
mod pack;
pub(crate) use index::{StoredIndex, catalog, delete_index, index_path};
pub(crate) use pack::{discard, installed, partial_bytes, remove_model, total};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use explorer::{NodeId, ScanOptions, is_binary_extension, is_image_extension, scan};

#[derive(Clone, Debug, Default)]
pub(crate) struct Results {
    pub hits: Vec<PathBuf>,
    pub indexed: usize,
    pub total: usize,
    pub failed: usize,
    pub last_error: Option<String>,
    pub complete: bool,
    pub query: String,
    pub unavailable: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Event {
    Status(String),
    Download(u64),
    Results(Results),
}

struct Entry {
    path: PathBuf,
    vectors: Vec<f32>,
}

struct Ranking {
    query: String,
    vector: Vec<f32>,
    top: Vec<(f32, usize)>,
}

impl Ranking {
    fn new(query: String, encoder: &mut encoder::Encoder, entries: &[Entry]) -> Result<Self> {
        let vector = encoder.query(&query)?;
        let mut ranking = Self {
            query,
            vector,
            top: Vec::new(),
        };
        for (i, entry) in entries.iter().enumerate() {
            ranking.add(i, entry);
        }
        Ok(ranking)
    }
    fn add(&mut self, i: usize, entry: &Entry) {
        let score = entry
            .vectors
            .as_chunks::<{ encoder::WIDTH }>()
            .0
            .iter()
            .map(|vec| {
                vec.iter()
                    .zip(&self.vector)
                    .map(|(a, b)| a * b)
                    .sum::<f32>()
            })
            .fold(f32::NEG_INFINITY, f32::max);
        let pos = self.top.partition_point(|(other, _)| *other >= score);
        if pos < 20 {
            self.top.insert(pos, (score, i));
            self.top.truncate(20);
        }
    }
    fn publish(&self, entries: &[Entry], stats: &Results, emit: &mut impl FnMut(Event)) {
        let hits = self
            .top
            .iter()
            .map(|(_, i)| entries[*i].path.clone())
            .collect();
        emit(Event::Results(Results {
            hits,
            query: self.query.clone(),
            ..stats.clone()
        }));
    }
}

pub(crate) struct Request {
    pub store: PathBuf,
    pub folder: Option<PathBuf>,
    pub query: String,
    pub show_ignored: bool,
    pub download: bool,
}

pub(crate) fn run(
    request: Request,
    cancel: &AtomicBool,
    queries: &Receiver<String>,
    mut emit: impl FnMut(Event),
) -> Result<()> {
    let dir = if request.download {
        pack::install(&request.store, cancel, |p| {
            emit(match p {
                pack::Progress::Download(bytes) => Event::Download(bytes),
                pack::Progress::Verify => Event::Status("Verifying model…".into()),
            });
        })?
    } else {
        ensure!(
            installed(&request.store),
            "Search model is missing. Open Content Search to download it."
        );
        request.store.join(pack::REVISION)
    };
    let Some(folder) = request.folder else {
        return Ok(());
    };
    emit(Event::Status("Preparing search…".into()));
    let _lease = pack::lease(&request.store)?;
    let mut encoder = encoder::Encoder::load(&dir)?;
    let mut entries = Vec::new();
    let mut ranking = Ranking::new(request.query, &mut encoder, &entries)?;
    let mut stats = Results::default();
    emit(Event::Status(format!(
        "Finding images and text in {}…",
        folder.display()
    )));
    let existed = index_path(&request.store, &folder).is_file();
    let index = index::Index::open(&request.store, &folder)?;
    let tree = scan(
        &folder,
        ScanOptions {
            show_ignored: request.show_ignored,
        },
        cancel,
    );
    match tree {
        Ok(tree) => {
            let mut pass = Inventory {
                index: &index,
                encoder: &mut encoder,
                entries: &mut entries,
                ranking: &mut ranking,
                stats: &mut stats,
            };
            pass.visit(&tree, queries, cancel, &mut emit)?;
            ensure!(!cancel.load(Ordering::Relaxed), "Paused");
            if let Err(e) = std::fs::read_dir(&folder) {
                stats.unavailable = Some(e.to_string());
            } else if tree
                .ids()
                .all(|id| tree.kind(id) != explorer::Kind::Unreadable)
            {
                index.finish_inventory()?;
            }
        }
        Err(explorer::ScanError::Root(error)) if existed => {
            stats.unavailable = Some(error.to_string());
        }
        Err(error) => return Err(error.into()),
    }
    if stats.unavailable.is_some() {
        entries = index.cached()?;
        ranking = Ranking::new(ranking.query, &mut encoder, &entries)?;
        stats.indexed = entries.len();
        stats.total = stats.indexed;
    }
    drop(index);
    stats.complete = true;
    ranking.publish(&entries, &stats, &mut emit);
    while let Ok(q) = queries.recv() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        ranking = Ranking::new(
            queries.try_iter().last().unwrap_or(q),
            &mut encoder,
            &entries,
        )?;
        ranking.publish(&entries, &stats, &mut emit);
    }
    Ok(())
}

struct Inventory<'a> {
    index: &'a index::Index,
    encoder: &'a mut encoder::Encoder,
    entries: &'a mut Vec<Entry>,
    ranking: &'a mut Ranking,
    stats: &'a mut Results,
}

impl Inventory<'_> {
    fn visit(
        &mut self,
        tree: &explorer::Tree,
        queries: &Receiver<String>,
        cancel: &AtomicBool,
        emit: &mut impl FnMut(Event),
    ) -> Result<()> {
        let Self {
            index,
            encoder,
            entries,
            ranking,
            stats,
        } = self;
        let files: Vec<_> = tree
            .files()
            .ones()
            .map(|n| NodeId::new(u32::try_from(n).expect("tree node IDs fit in u32")))
            .filter_map(|id| {
                let ext = Path::new(tree.name(id))
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                (tree.kind(id) == explorer::Kind::File
                    && !is_binary_extension(ext)
                    && !ext.eq_ignore_ascii_case("svg"))
                .then_some((id, is_image_extension(ext)))
            })
            .collect();
        stats.total = files.len();
        let mut last = Instant::now();
        for (id, picture) in files {
            ensure!(!cancel.load(Ordering::Relaxed), "Paused");
            if let Some(q) = queries.try_iter().last()
                && q != ranking.query
            {
                **ranking = Ranking::new(q, encoder, entries)?;
            }
            let path = tree.abs_path(id);
            match index.vectors(&path, |file| encoder.file(file, picture, cancel)) {
                Ok(vectors) => {
                    let entry = Entry { path, vectors };
                    ranking.add(entries.len(), &entry);
                    entries.push(entry);
                    stats.indexed += 1;
                }
                Err(error) => {
                    ensure!(!cancel.load(Ordering::Relaxed), "Paused");
                    stats.failed += 1;
                    stats.last_error = Some(format!("{}: {error}", path.display()));
                }
            }
            if stats.indexed + stats.failed == 1 || last.elapsed() >= Duration::from_millis(100) {
                ranking.publish(entries, stats, &mut *emit);
                last = Instant::now();
            }
        }
        Ok(())
    }
}

pub(crate) fn store() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("STRIKEBRAIN_VIEWER_SEARCH_DIR") {
        return Some(path.into());
    }
    Some(
        crate::persist::project_dirs()?
            .data_local_dir()
            .join("content-search"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "downloads the pinned real model into an explicit benchmark store"]
    fn install_benchmark_model() {
        let base =
            std::env::var_os("STRIKEBRAIN_VIEWER_SEARCH_DIR").expect("set an isolated model store");
        pack::install(Path::new(&base), &AtomicBool::new(false), |_| {}).unwrap();
    }
}
