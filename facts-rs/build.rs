// Embeds version info (product name, version, description) into facts-windows.exe.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "facts.")
            .set("FileDescription", "facts. offline AI launcher")
            .set("CompanyName", "facts. contributors")
            .set("LegalCopyright", "MIT License");
        res.compile().expect("embedding Windows version info failed (is the Windows SDK installed?)");
    }
}
