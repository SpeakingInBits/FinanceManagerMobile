fn main() {
    // One look on every platform (the app draws its own controls; this styles the date picker
    // and list scrollbars).
    let config = slint_build::CompilerConfiguration::new().with_style("material".into());
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("Slint UI should compile");
}
