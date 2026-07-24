# LawPDF for Android

This is a dependency-light native Android reader and markup app for PDF files, with a read-only DOCX view. It uses the Android Storage Access Framework, so it can open and save through local or cloud document providers without broad storage permissions.

## PDF controls

- Scroll vertically with one finger. In **Pan** mode, pinch smoothly from 100% to 400%, double-tap between fit and 200%, then drag horizontally across an enlarged page.
- Tap a page or the arrow at the top right to switch between the full app bar and a compact reader bar. The compact bar keeps **Open** reachable without covering the document.
- Tap **Edit tools** to reveal the horizontally scrollable markup palette; tap **Hide tools** when finished.
- **Pen**, **Highlight**, and **Sign** draw portable page-relative marks. **Color** cycles pen ink through navy, red, blue, and black.
- **Eraser**, **Undo**, and **Redo** make corrections before export. **Fit** returns to 100%.
- **Save copy** opens Android's create-document picker and writes a new flattened annotated PDF. The source document is never overwritten. Because the marked copy is flattened for portability without a third-party PDF library, its original text layer is not retained; keep the source PDF when selectable text matters.
- **Make default** opens Android's own PDF resolver using a harmless generated setup PDF. When Android shows the app list, choose LawPDF and **Always**. If another app already owns PDF files, LawPDF opens that app's settings first so its existing default can be cleared.

DOCX documents remain selectable, virtualized text views. PDF-only tools stay unavailable while a DOCX is open.

## Build without putting generated files in Box

Run from PowerShell:

```powershell
.\android\build.ps1 assembleDebug
```

The script puts all build output, the Gradle user home, and the per-project Gradle cache under `C:\tmp\lawpdf-android`. The debug APK is written to `C:\tmp\lawpdf-android\build\app\outputs\apk\debug\app-debug.apk`.

Do not run a bare Gradle command in this Box-synced checkout. If a different external drive is needed, set `LAWPDF_ANDROID_EXTERNAL_ROOT` first.

## Supported files

- PDF through Android's native `PdfRenderer`
- DOCX through a streaming Office Open XML text extractor; paragraphs, tables, tabs, and explicit line breaks are preserved as readable text

The app remains framework-native. Performance work and its validation are tracked in `PERFORMANCE.md`; UI and platform research is recorded in `UX_RESEARCH.md`.
