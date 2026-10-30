//! The iOS launcher: `azeroth_survivors_main`, called from the Xcode app's `main` on the main
//! thread (winit's UIKit event loop must own it), with the build stamp and the sandbox paths.
//!
//! The bundle is read-only, so the state folder goes to the app's `Documents/` unless
//! `$BENILLA_HOME` is set. The data is `<bundle>/Data` (the exe's own folder, the release layout
//! [`benilla_formats`] already searches) unless `$WOW_DATA` points elsewhere, as the simulator
//! scheme does to read the Mac's copy instead of a 5 GB bundle.

use benilla_app::BuildId;

/// The app's whole run; returns the process exit code.
#[no_mangle]
pub extern "C" fn azeroth_survivors_main() -> i32 {
    // A phone's panel is 3× its points (2868×1320 on a Pro Max): the world at 0.6 of that keeps a
    // sharp picture at a third of the pixels; the interface stays native. Through the environment
    // it is session-owned: a `renderScale` in `config.toml` does not apply (the mode has no
    // setting for it); a scheme's `WOW_RENDER_SCALE` replaces it.
    if std::env::var_os("WOW_RENDER_SCALE").is_none() {
        std::env::set_var("WOW_RENDER_SCALE", "0.6");
    }
    if std::env::var_os("BENILLA_HOME").is_none() {
        if let Some(home) = std::env::var_os("HOME") {
            let docs = std::path::Path::new(&home).join("Documents/benilla-config");
            std::env::set_var("BENILLA_HOME", docs);
        }
    }
    let exit = benilla_app::run_survivors(BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        project_dir: env!("BENILLA_PROJECT_DIR"),
        ..Default::default()
    });
    if exit.is_success() {
        0
    } else {
        1
    }
}
