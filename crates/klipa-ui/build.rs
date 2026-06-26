fn main() {
    // On Windows, embed the app icon + version metadata into klipa.exe.
    #[cfg(windows)]
    {
        let ico = "../../packaging/icons/klipa.ico";
        if std::path::Path::new(ico).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(ico);
            res.set("ProductName", "klipa");
            res.set("FileDescription", "klipa - clipboard manager");
            res.set("CompanyName", "Petros Dhespollari");
            res.set("LegalCopyright", "© 2026 Petros Dhespollari - MIT");
            if let Err(e) = res.compile() {
                println!("cargo:warning=winresource failed: {e}");
            }
        }
    }
}
