//! A pinned pack of data files. The signed executable supplies the URLs and
//! digests. A directory rename publishes all three files together.
mod shared;
pub(super) use shared::install;

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail, ensure};
use sha2::{Digest as _, Sha256};

use crate::update::download;

pub(super) const REVISION: &str = "f9972ac22e696a105c482202646979e33e4b5a1e";
const BASE: &str = "https://huggingface.co/google/embeddinggemma-2/resolve";
const BLOCK: usize = 64 * 1024;
const REPORT_BYTES: u64 = 1024 * 1024;

struct Asset {
    name: &'static str,
    size: u64,
    digest: &'static str,
}

const FILES: [Asset; 3] = [
    Asset {
        name: "config.json",
        size: 4455,
        digest: "b8f1e9931b57fbc054acdb445c41765d55b0074c58d145fa82839941ad1b5bb3",
    },
    Asset {
        name: "tokenizer.json",
        size: 32_170_510,
        digest: "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
    },
    Asset {
        name: "model.safetensors",
        size: 1_488_915_288,
        digest: "197a32965d4b1105faf060417baa899e193fb73cd401f42ec9295234d5553d79",
    },
];

pub(crate) fn total() -> u64 {
    FILES.iter().map(|file| file.size).sum()
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Progress {
    Download(u64),
    Verify,
}

pub(crate) fn installed(base: &Path) -> bool {
    base.join(REVISION).is_dir()
}

fn stage(base: &Path) -> PathBuf {
    base.join(format!("{REVISION}.partial"))
}

fn locked(base: &Path) -> Result<File> {
    fs::create_dir_all(base)?;
    let lock = File::create(base.join("install.lock"))?;
    lock.try_lock()
        .context("Another window is changing the search model")?;
    Ok(lock)
}

/// Bytes available for reuse when the person resumes an interrupted setup.
pub(crate) fn partial_bytes(base: &Path) -> Result<u64> {
    let dir = stage(base);
    let mut bytes = 0;
    for asset in FILES {
        let path = dir.join(asset.name);
        for file in [&path, &path.with_extension("part")] {
            match fs::metadata(file) {
                Ok(m) => bytes += m.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(bytes)
}

pub(super) fn lease(base: &Path) -> Result<File> {
    let lock = File::create(base.join("use.lock"))?;
    lock.lock_shared()?;
    Ok(lock)
}

pub(crate) fn remove_model(base: &Path) -> Result<()> {
    let _lock = locked(base)?;
    let readers = File::create(base.join("use.lock"))?;
    readers
        .try_lock()
        .context("Close active content searches before removing the model")?;
    let dest = base.join(REVISION);
    let removed = base.join(format!("{REVISION}.removed"));
    if removed.try_exists()? {
        fs::remove_dir_all(&removed)?;
    }
    fs::rename(dest, &removed)?;
    Ok(fs::remove_dir_all(removed)?)
}

pub(crate) fn discard(base: &Path) -> Result<()> {
    let _lock = locked(base)?;
    if stage(base).exists() {
        fs::remove_dir_all(stage(base))?;
    }
    Ok(())
}

fn install_files(
    base: &Path,
    cancel: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> Result<PathBuf> {
    let dest = base.join(REVISION);
    if dest.is_dir() {
        return Ok(dest);
    }
    let _lock = locked(base)?;
    if dest.is_dir() {
        return Ok(dest);
    }
    let staging = stage(base);
    fs::create_dir_all(&staging)?;
    let remaining = total().saturating_sub(partial_bytes(base)?);
    let free = fs2::available_space(base)?;
    ensure!(
        free >= remaining,
        "Free another {} MB to download the model",
        remaining.saturating_sub(free).div_ceil(1_000_000)
    );
    let agent = download::agent();
    let mut done = 0;
    for asset in &FILES {
        let path = staging.join(asset.name);
        ensure!(!cancel.load(Ordering::Relaxed), "Paused");
        if path.try_exists()? {
            report(Progress::Verify);
            verify(&path, asset, cancel)?;
        } else {
            let url = format!("{BASE}/{REVISION}/{}", asset.name);
            fetch(&agent, &url, asset, &path, cancel, |p| match p {
                Progress::Download(n) => report(Progress::Download(done + n)),
                Progress::Verify => report(p),
            })?;
        }
        done += asset.size;
    }
    fs::rename(staging, &dest)?;
    Ok(dest)
}

fn fetch(
    agent: &ureq::Agent,
    url: &str,
    asset: &Asset,
    dest: &Path,
    cancel: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> Result<()> {
    let partial = dest.with_extension("part");
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&partial)?;
    let mut size = file.metadata()?.len();
    ensure!(
        size <= asset.size,
        "Saved download exceeds the model size. Discard Download to restart."
    );
    let remaining = asset.size - size;
    let available = fs2::available_space(&partial)?;
    ensure!(
        available >= remaining,
        "Free another {} MB to resume",
        (remaining.saturating_sub(available)).div_ceil(1_000_000)
    );
    report(Progress::Download(size));
    if remaining > 0 {
        ensure!(!cancel.load(Ordering::Relaxed), "Paused");
        let mut request = download::get(agent, url)?.header("Accept-Encoding", "identity");
        if size > 0 {
            request = request.header("Range", format!("bytes={size}-"));
        }
        let mut response = request.call()?;
        if size > 0 {
            let content_range = format!("bytes {size}-{}/{}", asset.size - 1, asset.size);
            ensure!(
                response.status() == 206
                    && response
                        .headers()
                        .get("Content-Range")
                        .and_then(|x| x.to_str().ok())
                        == Some(content_range.as_str()),
                "Server could not resume the download. Discard Download to restart."
            );
        } else {
            ensure!(
                response.status() == 200,
                "Unexpected model response {}",
                response.status()
            );
        }
        let mut reader = response.body_mut().as_reader();
        let mut buffer = vec![0; BLOCK];
        let mut last = size;
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Paused");
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let next = size + read as u64;
            ensure!(
                next <= asset.size,
                "Model server sent more bytes than declared"
            );
            file.write_all(&buffer[..read])?;
            size = next;
            if size - last >= REPORT_BYTES || size == asset.size {
                report(Progress::Download(size));
                last = size;
            }
        }
        ensure!(size == asset.size, "Download interrupted at {size} bytes");
        file.sync_all()?;
    }
    drop(file);
    report(Progress::Verify);
    verify(&partial, asset, cancel)?;
    fs::rename(partial, dest)?;
    Ok(())
}

fn verify(path: &Path, asset: &Asset, cancel: &AtomicBool) -> Result<()> {
    let mut hash = Sha256::new();
    let mut input = File::open(path)?;
    let length = input.metadata()?.len();
    let mut buffer = vec![0; BLOCK];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Paused");
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    drop(input);
    if length != asset.size || hex::encode(hash.finalize()) != asset.digest {
        fs::remove_file(path)?;
        bail!(
            "{} failed verification. Resume to download {} MB again.",
            asset.name,
            asset.size.div_ceil(1_000_000)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::test_server::{Reply, Server};

    const TEST: Asset = Asset {
        name: "fixture.bin",
        size: 6,
        digest: "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721",
    };

    #[test]
    fn resumes_exact_range_and_verifies_the_entire_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(TEST.name);
        fs::write(dest.with_extension("part"), b"abc").unwrap();
        let server = Server::start();
        server.route("/model", Reply::partial(b"def", "bytes 3-5/6"));
        fetch(
            &download::agent(),
            &server.url("/model"),
            &TEST,
            &dest,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abcdef");
        assert_eq!(server.stop()[0].header("range"), Some("bytes=3-"));
    }

    #[test]
    fn full_or_misaligned_response_never_appends_to_retained_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(TEST.name);
        let part = dest.with_extension("part");
        fs::write(&part, b"abc").unwrap();
        for response in [Reply::ok(b"abcdef"), Reply::partial(b"def", "bytes 2-4/6")] {
            let server = Server::start();
            server.route("/model", response);
            fetch(
                &download::agent(),
                &server.url("/model"),
                &TEST,
                &dest,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap_err();
            assert_eq!(fs::read(&part).unwrap(), b"abc");
            assert!(!dest.exists());
        }
    }

    #[test]
    fn a_completed_partial_is_verified_locally_and_a_bad_digest_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(TEST.name);
        let part = dest.with_extension("part");
        let server = Server::start();
        fs::write(&part, b"abcdeg").unwrap();
        fetch(
            &download::agent(),
            &server.url("/model"),
            &TEST,
            &dest,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert!(!part.exists());
        assert!(!dest.exists());
        fs::write(&part, b"abcdef").unwrap();
        fetch(
            &download::agent(),
            &server.url("/model"),
            &TEST,
            &dest,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(server.stop().is_empty());
    }

    #[test]
    fn retained_verified_assets_are_checked_again_before_activation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TEST.name);
        for invalid in [b"abcdeg".as_slice(), b"abc"] {
            fs::write(&path, invalid).unwrap();
            verify(&path, &TEST, &AtomicBool::new(false)).unwrap_err();
            assert!(!path.exists());
        }
    }
}
