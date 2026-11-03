// Embeds the app icon (and version info) into airhome.exe on Windows.
fn main() {
    println!("cargo:rerun-if-changed=logos/airhome.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("logos/airhome.ico");
        res.set("ProductName", "airhome");
        res.set("FileDescription", "airhome - stream Windows audio to HomePod");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break the build; the exe just gets no icon.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
