//! Learning of a newer release and installing it.

// Tests use these; the app does not until the updater lands.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the updater in the next task")
)]
mod feed;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the updater in the next task")
)]
mod install;
