fn main() {
    // `links = "nd-swift-appkit"` hands this path to the build scripts of direct dependents.
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::metadata=swift_package={dir}/swift/NDSwift");
}
