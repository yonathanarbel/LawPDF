# LawPDF

**“The world's best PDF app for lawyers and law professors.”**  
That's the ambition behind LawPDF: a reader built around how legal scholars actually work.

Created by **Professor Yonathan Arbel**, LawPDF is a free, open-source desktop PDF
reader and annotation editor for Windows and Apple-silicon Macs.

Read the article, follow the footnotes, and keep your place. **Review Mode**
reconstructs reading flow and footnotes in law review articles, while the original
PDF remains available for checking quotations, citations, tables, and layout.
Highlight a passage for class, leave a comment on a colleague's draft, or work
through a case with search, annotation, and recovery tools in one application.

The “world's best” line expresses the creator's vision, not an independently
tested ranking. **Version 0.2.33 is a public beta.**

## Download

[**Download LawPDF for Windows or Mac**](https://github.com/yonathanarbel/LawPDF/releases/tag/v0.2.33)

| Platform | Download | Requirements |
| --- | --- | --- |
| Windows installer | [LawPDFSetup-x64.exe](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.33/LawPDFSetup-x64.exe) | 64-bit Windows; installation may request administrator approval |
| Windows portable | [LawPDF-windows-portable-x64.zip](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.33/LawPDF-windows-portable-x64.zip) | Extract the entire ZIP, then run `lawpdf.exe` |
| Mac | [LawPDF-macos.zip](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.33/LawPDF-macos.zip) | Apple silicon (M-series), macOS 13 or later; Intel Macs are not supported by this build |

Close older LawPDF versions before installing or opening the new version.
Keep a backup of important PDFs while trying the beta.

### Windows installation

Run the installer, or choose the portable ZIP if you prefer not to install.
The packages are not publisher-signed, so Windows may show an unknown-publisher
or reputation warning. Download only from this repository's release page.
The installer registers LawPDF as a PDF-capable app and offers the Windows
default-app chooser. You can also choose **Make LawPDF the default PDF
reader** from the **⋯** menu; Windows requires you to confirm the PDF
association yourself.

### Mac installation

Extract the ZIP and drag `LawPDF.app` to Applications. This build is ad hoc signed
and is **not notarized by Apple**. If macOS blocks the first launch, use Apple's
documented per-app approval process in **System Settings → Privacy & Security**
after attempting to open it. Do not disable Gatekeeper globally.
See [Apple's instructions](https://support.apple.com/en-us/102445) and our
[signing policy](docs/CODE_SIGNING.md).

## Built for legal reading, research, and teaching

- **Read footnote-heavy scholarship:** Review Mode reconstructs reading flow
  and sets each footnote in the margin beside the line that cites it. Long or
  crowded notes are shortened to the room they have; click a note, or its
  number in the text, to read it in full. Keep the source PDF close for
  verification.
- **Know what you are reading:** the masthead names the article, its authors,
  and its citation, read from the PDF itself, with contents, find, and the
  Original / Review / Side by side switch beneath.
- **Prepare for class and workshops:** highlight, underline, comment, add text
  boxes, and save annotated PDFs. The markup tools, zoom, page, and save status
  share one bar you can drag anywhere over the page.
- **Find the passage you need:** native text selection, search, copy,
  continuous multi-page viewing, and zoom.
- **Work with scanned materials:** OCR support for documents without native text.
- **Keep your work:** automatic annotation saving, visible save status,
  undo/redo, recovery copies, and external-file conflict detection. Closing
  waits for pending writes; failed writes keep the document open.
  Protected PDFs require Save As for an annotated copy.
- **Choose your tools:** optional AI-provider features, with provider keys stored
  in the operating system's credential store. Review the
  [privacy guide](docs/PRIVACY.md) before using network-backed features.

## What to expect from this beta

The Windows and Mac builds passed their automated regression and packaging
checks. The exact Windows installer was verified on a clean hosted Windows
machine. The Mac application has also undergone local runtime and annotation
checks. Version 0.2.33 fixes opening PDFs from Finder when LawPDF is closed,
closing the empty window with Command-W, and routing Mac Quit through the
save-or-cancel flow. See the [release verification](docs/releases/v0.2.33.md).

Broader screen-reader workflows, real-world reliability, large-document
performance, and signed-update replacement/recovery still need more testing.
Reading reconstruction can make mistakes: verify quotations and citations
against the original PDF, and keep backups of important work.

Updates require a manifest signed by LawPDF's pinned release key plus matching
package sizes and SHA-256 hashes. This is separate from Windows publisher
signing and Apple notarization. Until the complete automatic-update path is
qualified, the release-page downloads provide the manual installation route.

Found a problem? [Report an issue](https://github.com/yonathanarbel/LawPDF/issues).
Include your operating system, LawPDF version, and steps to reproduce it.
Do not post confidential client files or private research.

## Creator, license, and no warranty

**Created and maintained by Professor Yonathan Arbel.**

LawPDF is free and open source under the [MIT License](LICENSE).
**It is provided “AS IS,” without warranty of any kind, express or implied,
including merchantability, fitness for a particular purpose, and
noninfringement.** The license also limits the authors' liability.
No promise of error-free operation, document preservation, or suitability for a
particular legal matter is made. See the full license for its terms.

Bundled third-party components and models are credited in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Technical evidence and remaining qualification work are documented in
[the current release evidence](docs/releases/v0.2.33.md)
and [release procedure](docs/RELEASING.md).
