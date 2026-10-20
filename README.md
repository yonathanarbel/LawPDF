# LawPDF

**“The world's best PDF app for lawyers and law professors.”**  
That's the ambition behind LawPDF: a reader built around how legal scholars actually work.

Created by **Professor Yonathan Arbel**, LawPDF is a free, open-source desktop PDF
reader and annotation editor for Windows and Apple-silicon Macs.

The development build adds Linux x86_64 support; see [Linux setup](docs/LINUX.md).
Linux packages will be available with the next release.

Read the article, follow the footnotes, and keep your place. **Review Mode**
reconstructs reading flow and footnotes in law review articles, while the original
PDF remains available for checking quotations, citations, tables, and layout.
Highlight a passage for class, leave a comment on a colleague's draft, or work
through a case with search, annotation, and recovery tools in one application.

The development build also provides separate **Show footnotes** and **Show
comments** controls in Review Mode. Imported PDF comments retain their authors
and reply threads. Set your own comment name under **⋯ → Comment name…**, or
click your name on a comment to edit it globally. Author colors distinguish
readers. Narration uses **System**, **ElevenLabs**, or **OpenRouter** throughout
the app; select the source and configure provider keys in Settings.

Footnotes also offer **Find source**. Existing URLs and DOIs open directly;
otherwise BBGPT returns candidate article links in the note. Choose **Find
article links** in Review to queue up to 32 unresolved footnotes at a time.
Lookups run in the background, deduplicate identical queries, and cache results.
Only the selected footnote text is sent; opening a document starts no searches.

The “world's best” line expresses the creator's vision, not an independently
tested ranking. **Version 0.2.43 is a public beta.**

## Download

[**Download LawPDF for Windows or Mac**](https://github.com/yonathanarbel/LawPDF/releases/tag/v0.2.43)

| Platform | Download | Requirements |
| --- | --- | --- |
| Windows Store | [Microsoft Store](https://apps.microsoft.com/detail/9MZXSP0C423F) | Microsoft-signed package and Store-managed updates; check the version offered while updates await certification |
| Windows installer | [LawPDFSetup-x64.exe](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.43/LawPDFSetup-x64.exe) | 64-bit Windows; installation may request administrator approval |
| Windows portable | [LawPDF-windows-portable-x64.zip](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.43/LawPDF-windows-portable-x64.zip) | Extract the entire ZIP, then run `lawpdf.exe` |
| Mac | [LawPDF-macos.zip](https://github.com/yonathanarbel/LawPDF/releases/download/v0.2.43/LawPDF-macos.zip) | Apple silicon (M-series), macOS 13 or later; Intel Macs are not supported by this build |

Close older LawPDF versions before installing or opening the new version.
Keep a backup of important PDFs while trying the beta.

### Windows installation

Prefer the Microsoft Store edition for Microsoft-signed installation and
automatic Store updates. The Store edition follows a separate release process;
check the version offered there. A GitHub release does not establish Store availability.

For direct downloads, run the installer or choose the portable ZIP.
These direct-download packages are not publisher-signed, so Windows may show an unknown-publisher
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
- **Follow the page into Review Mode:** a brief reveal shows its words moving
  into the reading column and footnote cards on the first switch. Review Mode
  opens at your current page. Reduced motion skips the animation, and any
  click, scroll, or key press ends it.
- **Know what you are reading:** the masthead names the article, its authors,
  and its citation, read from the PDF itself, with contents, find, and the
  Original / Review / Side by side switch beneath.
- **Prepare for class and workshops:** highlight, underline, comment, add text
  boxes, and save annotated PDFs. The markup tools, zoom, page, and save status
  share one bar you can drag anywhere over the page.
- **Print on Mac:** press **Command-P** or choose **File → Print…** for the
  standard print dialog, page ranges, and Save as PDF. Printing includes current
  annotations without changing the original file.
- **Find the passage you need:** native text selection, search, copy,
  continuous multi-page viewing, and zoom.
- **Work with scanned materials:** **Run OCR** reads scanned pages with the text
  recognizer built into Windows or macOS, so there is nothing extra to install
  and nothing leaves your computer. Recognized text feeds search, copying, and
  Review Mode, and **OCR: save a searchable copy** writes a searchable PDF.
  On Windows, OCR uses the recognition languages installed with Windows; if none
  is installed, LawPDF says how to add one. **Cloud OCR** through your own
  OpenRouter key remains optional.
- **Keep your work:** automatic annotation saving, visible save status,
  undo/redo, recovery copies, and external-file conflict detection. Closing
  waits for pending writes; failed writes keep the document open.
  Protected PDFs require Save As for an annotated copy.
- **Choose where to save:** use **Save As…** in the toolbar or the Mac **File**
  menu (⌘⇧S on Mac, Ctrl+Shift+S on Windows). Choose a folder and filename;
  further edits save to that file. **File → Show in Finder** locates the current
  PDF on Mac, and hovering over the save status shows its full path.
  **Export → Save PDF copy** makes a separate copy while keeping the current
  document open at its original location.
- **Choose your tools:** optional AI-provider features, with provider keys stored
  in the operating system's credential store. Review the
  [privacy guide](docs/PRIVACY.md) before using network-backed features.

## What to expect from this beta

The Windows and Mac builds passed their automated regression and packaging
checks. The exact Windows installer was verified on a clean hosted Windows
machine. Version 0.2.43 adds the Review Mode reveal and keeps reporter citation
digits out of footnote callouts. It includes on-device OCR and searchable
copies, Save As, Mac printing, annotation recovery, and the earlier Finder
opening and save-aware closing fixes. See the
[release verification](docs/releases/v0.2.43.md) for evidence and its scope.

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

### A little coffee, a lot of gratitude

LawPDF is free. If you enjoy using it and want to support my coffee addiction,
you can [buy me a coffee](https://buymeacoffee.com/lawpdf). A little coffee keeps
the footnotes flowing. Support is entirely optional and does not unlock features.
The small coffee cup in the app's top bar opens the same support page in your browser.

### License

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
[the current release evidence](docs/releases/v0.2.43.md)
and [release procedure](docs/RELEASING.md).
