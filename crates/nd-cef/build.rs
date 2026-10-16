fn main() {
    // Where cef-dll-sys unpacked the distribution, for dev runs outside an app bundle.
    let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") else { return };
    println!("cargo::rustc-env=ND_CEF_BUILD_DIR={dir}");
    // Linux links libcef.so: this crate's own tests and helper load it from there.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
