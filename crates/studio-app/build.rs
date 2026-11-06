fn main() {
    println!("cargo:rerun-if-changed=exaudio.rc");
    println!("cargo:rerun-if-changed=../../assets/app/exaudio.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Extension views compose several GPUI render frames. Windows' default 1 MiB
        // main stack is insufficient in debug builds; reserve 8 MiB, committed on demand.
        let stack_arg = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            "/STACK:8388608"
        } else {
            "-Wl,--stack,8388608"
        };
        println!("cargo:rustc-link-arg-bin=studio-app={stack_arg}");
        // Icon resource id 1 is what gpui loads for the window class and the tray loads by ordinal.
        // Required: a Windows build without a resource compiler must fail, not ship an iconless exe.
        embed_resource::compile("exaudio.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
