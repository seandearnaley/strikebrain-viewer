# Browsing a network folder during local content indexing

On an M4 Max, indexing 50 PNGs on an SMB share took 19.29–19.50 seconds. The first matching file appeared in 1.38–1.41 seconds. Opening a new viewer window and searching the unchanged folder took 0.66–0.68 seconds from Return to all 50 files being searchable. This includes opening the model and checking the files’ metadata against the local index.

## Preview latency

Each cell gives the median and 95th percentile in milliseconds for 120 keyboard previews. Paired p95 differences between indexing and idle-before ranged from −0.63 to +0.31 ms. The experiment cannot resolve a consistent indexing penalty on this workload. [Sorted samples in microseconds](2026-11-28-content-search-samples.csv)

| Run | Idle before | Indexing | Idle after | First result | All 50 | Reopen |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 7.76 / 8.93 | 7.14 / 8.30 | 6.98 / 7.90 | 1.384 s | 19.419 s | 0.663 s |
| 2 | 6.94 / 8.04 | 7.29 / 8.35 | 7.55 / 8.27 | 1.400 s | 19.291 s | 0.676 s |
| 3 | 7.39 / 8.20 | 7.52 / 8.41 | 7.76 / 8.59 | 1.405 s | 19.498 s | 0.683 s |

The read-only corpus was one folder of 50 PNGs totaling 141,540,813 bytes. Each run started with an installed model and an empty local index. The completed SQLite database occupied 241,664 bytes. The machine had an Apple M4 Max, 128 GiB of memory and macOS 27.0.1. The test used the workspace development profile with optimized dependencies. The OS, GPU and SMB caches were uncontrolled.

The harness opens a 1000 × 700 GPUI test window. It issues Up and Down through the explorer, waits for the UI executor, renders the frame and checks that the stage selection actually moved. It cycles over five images and discards four warm-up keystrokes per phase. All three active samples began at 1/50 files indexed and ended at 3/50. The 95th percentile is the 114th sorted sample. First-result and completion times start when the simulated consent click returns. The warm run closes the window, opens a new one with the same store and submits the same query.

These timings cover repeated keyboard previews on GPUI’s offscreen test platform. Display presentation, first visits to 120 distinct images, sustained memory pressure, indexing an entire share and CPU-only inference need separate measurements. The machine fetched and verified the initial 1.52 GB model in approximately 15 seconds in a separate run. Download time depends on the connection and host.

## Reproduce

Choose a folder with at least five directly listed images and enough files to keep indexing active for a full preview sample. Use a fresh local store per run. These commands fetch the model before the timers start and write the observations to JSON.

```sh
export EXPLORER_SCAN_ROOT="/path/to/network/folder"
export STRIKEBRAIN_VIEWER_SEARCH_DIR="$(mktemp -d)"
export STRIKEBRAIN_BENCH_RESULT="$STRIKEBRAIN_VIEWER_SEARCH_DIR/timings.json"
cargo test -p viewer --lib install_benchmark_model -- --ignored
cargo test -p viewer --test ui browsing_during_content_index -- --ignored --nocapture
```

The opt-in semantic smoke test uses a temporary source and model store. It enters through the first-use UI, downloads the real files and checks retrieval through the preview. The regular test suite uses local fixtures and can run without model access. [UI and benchmark harness](../../crates/viewer/tests/ui/search_bench.rs)

```sh
cargo test -p viewer --test ui fresh_download_finds_semantic_matches_and_previews_them -- --ignored
```

## What is indexed

The viewer uses the Rust Lattice encoder with Google's published weights. It selects the Metal backend on Apple silicon and CPU on the other release targets. Glypr 0.4.0 also supported the intended retrieval flow, but its mandatory Hub client introduced a CA bundle whose license the repository's dependency policy rejects. Lattice reads the three verified local files directly. The app owns network access and uses its existing HTTPS client for the download. The pinned manifest totals 1,521,090,253 bytes. [Google model revision](https://huggingface.co/google/embeddinggemma-2/tree/f9972ac22e696a105c482202646979e33e4b5a1e) · [Lattice encoder](https://docs.rs/lattice-model-embeddinggemma2/0.18.0/lattice_model_embeddinggemma2/) · [Manifest](../../crates/viewer/src/search/pack.rs)

A dedicated worker indexes raster images and UTF-8 text within the viewer's size limits. It uses Google's retrieval query and document prompts for text, divides documents into UTF-8-safe passages of up to 1,024 bytes and ranks each file by its best passage. An image contributes one embedding. The UI returns up to 20 files by exact dot product over normalized 256-dimensional vectors. Activating a match opens the file. This version indexes still images and text. Audio, video and jumping to the matched passage are further work. [Google model overview](https://blog.google/innovation-and-ai/technology/developers-tools/embeddinggemma-2/) · [Encoder](../../crates/viewer/src/search/encoder.rs)

A completed file is checkpointed with its size and modification time. An unchanged file reuses its stored vectors on the next search. Equal-length edits with a preserved modification time can evade that check. The worker holds vectors for the current scope in memory to rerank without reading the source again. Large text libraries and simultaneous searches in multiple windows can therefore have higher peak memory than this image sample.
