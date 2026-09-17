# LawPDF

LawPDF is a desktop PDF reader and annotation editor designed for legal reading.

Its **Review Mode** reconstructs reading order and footnotes for law review articles.
The original PDF remains available for checking citations, tables, and any text
whose reconstruction is uncertain.

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
  macOS 13 or later (required by the bundled PDF engine).

Extract the ZIP and move `LawPDF.app` to Applications. This beta uses ad hoc signing and is not notarized by Apple. macOS may require
you to explicitly approve the first launch. GitHub distribution does not remove
that macOS warning. No paid signing membership is bundled or required to download
LawPDF. See the [signing policy](docs/CODE_SIGNING.md) for the current status.

LawPDF offers a small, one-time prompt when it is not your default PDF reader.
You can also change the association later with **Set as default** in the LawPDF
toolbar.

New automatic updates require an Ed25519 signature from LawPDF's pinned release
key, the exact package size, and its SHA-256 checksum. This authenticates the
update separately from Apple or Windows publisher signing. Older releases without
a signed manifest are available through the GitHub download link. Updates wait
for document saves before restarting.

## Features

- Review Mode for comfortable law review reading.
- Optional Windows and macOS default-PDF-reader integration.
- Continuous multi-page PDF viewing and zoom.
- Native text selection, search, and copy.
- Highlights, underlining, comments, free-text boxes, and signatures.
- Automatic annotation saving, with a visible saved/unsaved status. Closing waits
  for pending writes; failed writes keep the document open. Protected PDFs require
  Save As to make an annotated copy.
- Recovery copies for interrupted editing, external-file conflict detection, and
  annotation undo/redo. A recovery dialog lets you export a separate marked PDF.
- Provider keys stored in the operating system credential store, configurable
  local cache retention, and support diagnostics that exclude document contents.
- OCR support for scanned documents.
- Footnote navigation and reading-flow reconstruction.
- Export and save tools for annotated documents.

## License and notices

LawPDF is released under the [MIT License](LICENSE). Bundled third-party
components and models are documented in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Privacy and release readiness

Read the [privacy guide](docs/PRIVACY.md), [release procedure](docs/RELEASING.md),
and [production completion plan](docs/PRODUCTION_PLAN.md). A successful local
build alone does not establish Windows, Android, or assistive-technology readiness.
Close older LawPDF versions before opening a newly installed version.
