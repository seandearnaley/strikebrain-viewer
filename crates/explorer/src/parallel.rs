//! Independent work spread over threads.

use std::cmp::Ordering;
use std::num::NonZeroUsize;

/// Elements to sort before a second thread is worth starting.
const PARALLEL_SORT_MIN: usize = 1 << 14;

/// Sorts each slice by `compare`. Slices are independent, so large
/// workloads spread over threads in contiguous runs of about equal length.
pub(crate) fn sort_each<T: Send>(
    slices: Vec<&mut [T]>,
    compare: &(impl Fn(&T, &T) -> Ordering + Sync),
) {
    let total: usize = slices.iter().map(|slice| slice.len()).sum();
    let threads = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let run_len = total.div_ceil(threads).max(PARALLEL_SORT_MIN);
    let sort_run = |run: Vec<&mut [T]>| {
        for slice in run {
            slice.sort_unstable_by(compare);
        }
    };
    std::thread::scope(|scope| {
        let mut run = Vec::new();
        let mut run_size = 0;
        for slice in slices {
            run_size += slice.len();
            run.push(slice);
            if run_size >= run_len {
                let full = std::mem::take(&mut run);
                run_size = 0;
                scope.spawn(move || sort_run(full));
            }
        }
        sort_run(run);
    });
}
