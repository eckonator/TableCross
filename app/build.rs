//! On Windows the program icon and version information go into the .exe.

fn main() {
    // Only when building on Windows for Windows (needs rc.exe there); checks from other systems skip it
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../assets/tablecross.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/tablecross.ico");
        res.set("ProductName", "TableCross");
        res.set("FileDescription", "TableCross");
        res.set("LegalCopyright", "© 2026 Markus Eckert · GPL-3.0-or-later");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
