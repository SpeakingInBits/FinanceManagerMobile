//! Finance Tracker app: a Slint UI over `finance-core`, for iOS, Android and desktop.

mod controller;
mod viewmodel;

use std::path::PathBuf;

slint::include_modules!();

const DATA_FILE: &str = "finance-tracker.json";

/// Creates the window, loads data from `data_dir`, and runs the event loop until the app exits.
/// `on_dark_theme` lets the platform follow the app's light/dark theme.
pub fn run(
    data_dir: PathBuf,
    on_dark_theme: impl Fn(bool) + 'static,
) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let _controller = controller::attach(&ui, data_dir.join(DATA_FILE), on_dark_theme);
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
    let activity = app.clone();
    slint::android::init(app).expect("Slint Android backend should initialize");
    run(data_dir, move |dark| {
        set_light_system_bars(&activity, !dark)
    })
    .expect("app should run");
}

/// Matches the status and navigation bar icons to the app's theme via `MainActivity`. A failure
/// only leaves the icons following the system theme, so it is ignored.
#[cfg(target_os = "android")]
fn set_light_system_bars(app: &slint::android::AndroidApp, light: bool) {
    use jni::objects::JObject;
    use jni::{JValue, JavaVM, jni_sig, jni_str};

    let Ok(vm) = JavaVM::singleton() else { return };
    let _ = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let ptr = app.activity_as_ptr().cast();
        // Safety: android-activity hands out a valid global reference to the activity.
        let activity = unsafe { env.as_cast_raw::<jni::refs::Global<JObject>>(&ptr)? };
        let result = env.call_method(
            activity.as_ref(),
            jni_str!("setLightSystemBars"),
            jni_sig!("(Z)V"),
            &[JValue::Bool(light)],
        );
        if result.is_err() {
            env.exception_clear();
        }
        Ok(())
    });
}
