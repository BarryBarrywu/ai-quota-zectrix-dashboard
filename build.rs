fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/antigravity/desktop.m");
        cc::Build::new()
            .file("src/antigravity/desktop.m")
            .flag("-fobjc-arc")
            .compile("antigravity_desktop");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
    }
}
