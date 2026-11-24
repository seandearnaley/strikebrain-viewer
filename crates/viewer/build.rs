//! On Windows, compiles `bundle/windows.rc` into the executable: its icon,
//! which GPUI also gives the window (it loads icon resource 1), and the
//! version details Explorer shows. Elsewhere it does nothing.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=bundle/windows.rc");
    println!("cargo::rerun-if-changed=bundle/icon.ico");
    #[cfg(windows)]
    embed_resource::compile(
        "bundle/windows.rc",
        embed_resource::ParamsMacrosAndIncludeDirs(
            [
                concat!("VERSION=\"", env!("CARGO_PKG_VERSION"), "\""),
                concat!("VERSION_MAJOR=", env!("CARGO_PKG_VERSION_MAJOR")),
                concat!("VERSION_MINOR=", env!("CARGO_PKG_VERSION_MINOR")),
                concat!("VERSION_PATCH=", env!("CARGO_PKG_VERSION_PATCH")),
            ],
            ["bundle"],
        ),
    )
    .manifest_required()
    .expect("the resource compiler accepts bundle/windows.rc");
}
