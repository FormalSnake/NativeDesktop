fn main() {
    // Builds and links the SwiftPM package on macOS; does nothing on other targets.
    nd::build::swift_package("swift/Widgets");
}
