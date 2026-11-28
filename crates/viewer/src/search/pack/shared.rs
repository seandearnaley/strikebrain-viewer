//! Join an installation already in progress in another window. Only the
//! owner performs IO. Subscribers receive the same byte counts and result.
use super::Progress;
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, btree_map::Entry};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};

type Notice = Result<Progress, String>;
#[derive(Default)]
struct Observers {
    last: Option<Progress>,
    listeners: Vec<mpsc::Sender<Notice>>,
}

static ACTIVE: Mutex<BTreeMap<PathBuf, Observers>> = Mutex::new(BTreeMap::new());
fn active() -> MutexGuard<'static, BTreeMap<PathBuf, Observers>> {
    ACTIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Owner<'a>(&'a Path);
impl Drop for Owner<'_> {
    fn drop(&mut self) {
        active().remove(self.0);
    }
}

pub(in crate::search) fn install(
    base: &Path,
    cancel: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> Result<PathBuf> {
    let joined = match active().entry(base.to_owned()) {
        Entry::Occupied(mut entry) => {
            let observers = entry.get_mut();
            let (tx, rx) = mpsc::channel();
            if let Some(p) = observers.last {
                report(p);
            }
            observers.listeners.push(tx);
            Some(rx)
        }
        Entry::Vacant(entry) => {
            entry.insert(Observers::default());
            None
        }
    };
    if let Some(updates) = joined {
        for update in updates {
            ensure!(!cancel.load(Ordering::Relaxed), "Paused");
            report(update.map_err(anyhow::Error::msg)?);
        }
        ensure!(super::installed(base), "Model installation did not finish");
        return Ok(base.join(super::REVISION));
    }
    let _owner = Owner(base);
    let result = super::install_files(base, cancel, |p| {
        if let Some(observers) = active().get_mut(base) {
            observers.last = Some(p);
            observers
                .listeners
                .retain(|listener| listener.send(Ok(p)).is_ok());
        }
        report(p);
    });
    if let Err(e) = &result
        && let Some(observers) = active().get_mut(base)
    {
        observers
            .listeners
            .retain(|listener| listener.send(Err(format!("{e:#}"))).is_ok());
    }
    result
}
