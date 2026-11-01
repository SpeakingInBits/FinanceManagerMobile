//! Finance Tracker app: a Slint UI over `finance-core`, for iOS, Android and desktop.

mod controller;
mod viewmodel;

use std::path::PathBuf;

slint::include_modules!();

const DATA_FILE: &str = "finance-tracker.json";

/// Creates the window, loads data from `data_dir`, and runs the event loop until the app exits.
pub fn run(data_dir: PathBuf) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let _controller = controller::attach(&ui, data_dir.join(DATA_FILE));
    ui.run()
}

/// The per-user app data directory on desktop.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("FinanceTracker")
}

/// Application Support inside the app's sandbox container.
#[cfg(target_os = "ios")]
pub fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("Library/Application Support")
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    // App-private storage: removed with the app, never visible to other apps.
    let data_dir = app
        .internal_data_path()
        .unwrap_or_else(|| PathBuf::from("."));
    slint::android::init(app).expect("Slint Android backend should initialize");
    run(data_dir).expect("app should run");
}
