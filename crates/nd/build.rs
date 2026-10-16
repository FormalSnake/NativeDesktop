fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // Build scripts only see the metadata of their direct dependencies, so `nd` forwards the
    // framework's Swift package path for the app's `nd::build::app()`.
    if let Ok(path) = std::env::var("DEP_ND_SWIFT_APPKIT_SWIFT_PACKAGE") {
        println!("cargo::metadata=appkit_swift_package={path}");
    }
    // The appearance audit lays whole apps out on the main thread, which Windows gives 1 MB of
    // stack (8 on macOS and Linux); apps get the same from `nd::build::app()`.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo::rustc-link-arg-tests=/STACK:8388608");
    }
}
