fn main() {
    // zxing-cpp is static C++; Android has no system libstdc++, so link the NDK's static libc++.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=static=c++_static");
        println!("cargo:rustc-link-lib=static=c++abi");
    }
}
