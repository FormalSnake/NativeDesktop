fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // Build scripts only see the metadata of their direct dependencies, so `nd` forwards the
    // framework's Swift package path for the app's `nd::build::app()`.
    if let Ok(path) = std::env::var("DEP_ND_SWIFT_APPKIT_SWIFT_PACKAGE") {
        println!("cargo::metadata=appkit_swift_package={path}");
    }
}
