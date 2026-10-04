fn main() {
    let cfg = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".to_string());
    slint_build::compile_with_config("ui/main.slint", cfg)
        .expect("Slint UI derleme hatasi");

    // Windows'ta release'de konsol penceresi acmasin
    #[cfg(all(windows, not(debug_assertions)))]
    {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
        println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
    }
}
