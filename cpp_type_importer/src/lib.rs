#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn CorePluginInit() -> bool {
    Logger::new("DWARF Export")
        .with_level(LevelFilter::Debug)
        .init();
    // Initialize logging
    // Register custom architectures, workflows, demanglers,
    // function recognizers, platforms and views!
    true
}
