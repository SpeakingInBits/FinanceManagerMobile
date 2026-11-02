fn main() {
    // One look on every platform (the app draws its own controls; this styles the date picker
    // and list scrollbars).
    let config = slint_build::CompilerConfiguration::new().with_style("material".into());
    // The UI tests (src/ui_tests.rs) find elements by id and label, which needs the debug info.
    // They run on desktop, so the phone builds leave it out.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let config = config.with_debug_info(target_os != "android" && target_os != "ios");
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("Slint UI should compile");
}
