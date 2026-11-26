//! On Windows, compiles `bundle/windows.rc` into the executable: the brand
//! pack's icon, which GPUI also gives the window (it loads icon resource 1),
//! and the version details Explorer shows. Elsewhere it does nothing.

/// The brand pack's Windows icons.
const ICONS: &str = "../../brand/Strikebrain-Production-Pack/platform/windows";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=bundle/windows.rc");
    println!("cargo::rerun-if-changed={ICONS}/Strikebrain.ico");
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
            ["bundle", ICONS],
        ),
    )
    .manifest_required()
    .expect("the resource compiler accepts bundle/windows.rc");
}
