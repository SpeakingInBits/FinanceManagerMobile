// Desktop and iOS entry point; Android starts in `android_main` instead.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> Result<(), slint::PlatformError> {
    // `--data-dir <path>` points the desktop build at another data folder (handy for testing).
    let mut args = std::env::args().skip(1);
    let mut data_dir = None;
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            data_dir = args.next().map(PathBuf::from);
        }
    }
    finance_app::run(data_dir.unwrap_or_else(finance_app::default_data_dir))
}
