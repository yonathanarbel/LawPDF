# LawPDF

LawPDF is a desktop PDF reader and annotation editor designed for legal reading.

Its **Review Mode** converts law review articles to a smooth reading experience,
preserving the article and footnotes while removing page furniture such as
repeated headers, footers, and page numbers.

## Download

Download the newest version from the
[LawPDF releases page](https://github.com/yonathanarbel/LawPDF/releases/latest).

### Windows

- `LawPDFSetup-x64.exe` — standard 64-bit Windows installer.
- `LawPDF-windows-portable-x64.zip` — portable version; extract it and run
  `lawpdf.exe`.

The installer registers LawPDF as a PDF-capable app and offers to open the
Windows default-app chooser. You can reopen that chooser at any time with
**Set as default** in the LawPDF toolbar. Windows requires you to confirm the
`.pdf` association yourself.

### macOS

- `LawPDF-macos.zip` — application bundle for Apple-silicon Macs running
  macOS 12 or later.

Extract the ZIP and move `LawPDF.app` to Applications. Because this release is
not notarized through the Mac App Store, the first launch may require
Control-clicking the app, choosing **Open**, and confirming once.

LawPDF offers a small, one-time prompt when it is not your default PDF reader.
You can also change the association later with **Set as default** in the LawPDF
toolbar.

Automatic updates show download progress, verify the release checksum and app
signature, restart LawPDF to install, and confirm the installed version in a
top-right status card.

## Features

- Review Mode for comfortable law review reading.
- Optional Windows and macOS default-PDF-reader integration.
- Continuous multi-page PDF viewing and zoom.
- Native text selection, search, and copy.
- Highlights, underlining, comments, free-text boxes, and signatures.
- OCR support for scanned documents.
- Footnote navigation and reading-flow reconstruction.
- Export and save tools for annotated documents.

## License and notices

LawPDF is released under the [MIT License](LICENSE). Bundled third-party
components and models are documented in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
