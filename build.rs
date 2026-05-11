fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/lawpdf.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/lawpdf.ico")
            .set("ProductName", "LawPDF")
            .set("FileDescription", "LawPDF PDF reader and editor")
            .set("CompanyName", "Y. Arbel design (2026)")
            .set("LegalCopyright", "Copyright (c) 2026 Y. Arbel")
            .set("OriginalFilename", "lawpdf.exe")
            .set("InternalName", "lawpdf.exe");
        resource.compile()?;
    }

    Ok(())
}
