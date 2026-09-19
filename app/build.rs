//! The commit the program is built from and whether it is a release (for the update check); on
//! Windows also the program icon and version information in the .exe.

fn git(args: &[&str]) -> String {
    std::process::Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rustc-env=TC_GIT_HASH={}", git(&["rev-parse", "HEAD"]));
    println!(
        "cargo:rustc-env=TC_GIT_TIME={}",
        git(&["log", "-1", "--format=%ct", "HEAD"])
    );
    // TC_CHANNEL=release for builds of a release tag (set by the release workflow), else test build
    let channel = std::env::var("TC_CHANNEL").unwrap_or_default();
    let channel = if channel == "release" { "release" } else { "test" };
    println!("cargo:rustc-env=TC_CHANNEL={channel}");
    println!("cargo:rerun-if-env-changed=TC_CHANNEL");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");

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
