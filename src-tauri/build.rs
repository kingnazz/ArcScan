//! Build script.
//!
//! The one thing it does beyond the default is choose which capability set the
//! window gets, because the two editions do not have the same plugins. A
//! portable build does not link `tauri-plugin-updater` or
//! `tauri-plugin-process`, so a capability file naming `updater:default` would
//! not merely be redundant there -- it would fail the build, which is a fair
//! description of what it should do.
//!
//! Keeping them in separate directories rather than one file with conditional
//! entries means the granted permissions for each edition are readable as a
//! list, which is the point of a capability file.

fn main() {
    let portable = std::env::var_os("CARGO_FEATURE_PORTABLE").is_some();
    let pattern = if portable {
        "./capabilities/portable/*"
    } else {
        "./capabilities/installed/*"
    };

    // tauri-build emits no rerun-if-changed for a custom capabilities path.
    println!("cargo:rerun-if-changed=capabilities");

    tauri_build::try_build(tauri_build::Attributes::new().capabilities_path_pattern(pattern))
        .expect("failed to run tauri-build");
}
