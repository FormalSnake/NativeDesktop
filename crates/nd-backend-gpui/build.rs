fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The GPUI gates lay trees out on the main thread, which Windows gives 1 MB of stack (8 on
    // macOS and Linux).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo::rustc-link-arg-tests=/STACK:8388608");
    }
}
