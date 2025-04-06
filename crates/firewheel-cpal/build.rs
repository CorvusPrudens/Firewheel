fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "android" {
        // This includes 'lib/<arch>/libc++_shared.so'
        // Firewheel needs it currently on Android builds
        println!("cargo:rustc-link-lib=dylib=c++_shared");
    }
}
