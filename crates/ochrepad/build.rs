//! On Windows, embeds the logo as the executable's icon (what Explorer and
//! shortcuts show) along with version information from Cargo.toml. A
//! failure is a warning, not an error: the program works without an icon.

#[path = "src/logo.rs"]
#[allow(dead_code)]
mod logo;

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_icon();
    }
}

#[cfg(windows)]
fn embed_icon() {
    let Some(out) = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let path = out.join("ochrepad.ico");
    if let Err(e) = std::fs::write(&path, logo::ico(&[16, 20, 24, 32, 40, 48, 64, 256])) {
        println!("cargo:warning=could not write the icon: {e}");
        return;
    }
    let Some(path) = path.to_str() else {
        println!("cargo:warning=icon path is not UTF-8; no icon embedded");
        return;
    };
    let mut res = winresource::WindowsResource::new();
    res.set_icon(path);
    res.set("FileDescription", "ochrepad");
    res.set("ProductName", "ochrepad");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed the icon: {e}");
    }
}
