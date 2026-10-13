/// The app's build script entry (ARCHITECTURE.md 3.9). Aggregating and linking Swift packages
/// lands with the AppKit host lane; until then it only reruns on manifest changes.
pub fn app() {
    println!("cargo:rerun-if-changed=nativedesktop.toml");
}
