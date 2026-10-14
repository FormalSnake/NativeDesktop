fn main() {
    // Where cef-dll-sys unpacked the distribution, for dev runs outside an app bundle.
    if let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") {
        println!("cargo::rustc-env=ND_CEF_BUILD_DIR={dir}");
    }
}
