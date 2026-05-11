# LawPDF

LawPDF is a native Windows PDF reader and annotation editor built in Rust with `egui`.

## Features

- Open and render PDF documents with continuous multi-page scrolling.
- Zoom with toolbar controls or `Ctrl` + mouse wheel.
- Select native PDF text and copy with `Ctrl+C`.
- Highlight selected text with pastel markers or crimson underline.
- Add, edit, move, and delete FreeText text boxes.
- Draw e-signature ink annotations.
- Search native PDF text and OCR text.
- Run OCR in a background worker using the `tesseract` command line tool.
- Save an edited PDF copy.
- Export the current page to PNG.
- Export extracted native and OCR text to TXT.
- Tabbed sidebar with page thumbnails, search/OCR, outline, and notes.

## Downloads

Tagged releases are built by GitHub Actions and publish two Windows assets:

- `LawPDF-windows-portable-x64.zip`: portable folder containing `lawpdf.exe`, `pdfium.dll`, fonts, and notices.
- `LawPDFSetup-x64.exe`: Windows installer.

The portable build does not require installation. Extract the zip and run `lawpdf.exe`.

## Runtime Requirements

PDF rendering uses PDFium through `pdfium-render`. Release builds bundle `pdfium.dll`.

If you build from source, LawPDF checks for `pdfium.dll` in these places:

- beside the executable
- the current working directory
- `vendor\pdfium.dll` in the project
- the path in `PDFIUM_DYNAMIC_LIB_PATH`

OCR uses the `tesseract` command line program. Install Tesseract separately and make sure `tesseract.exe` is on `PATH`.

The UI uses EB Garamond from `fonts\EBGaramond.ttf` in release packages or `vendor\fonts\EBGaramond.ttf` in source builds.

## Build

```powershell
cargo build --release
```

To assemble a portable Windows package locally:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\package-windows.ps1 -Configuration release
```

To run a smoke test against a local PDF:

```powershell
$env:LAWPDF_SMOKE_PDF = "C:\path\to\sample.pdf"
cargo run -- --smoke-render-worker
```

For local development only, set `LAWPDF_DEFAULT_PDF` to auto-open a document on startup.

## Licensing

LawPDF is released under the MIT License. Third-party dependency and bundled binary notices are documented in `THIRD_PARTY_NOTICES.md` and `THIRD_PARTY_RUST_LICENSES.csv`.

The bundled PDFium binary comes from the `bblanchon/pdfium-binaries` package and is accompanied by its license and included third-party notices under `third_party/pdfium-binaries`.

EB Garamond is distributed under the SIL Open Font License 1.1.
