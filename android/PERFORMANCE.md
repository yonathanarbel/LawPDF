# Android performance iterations

## Iteration 1: functional reader

The first build opened PDF and DOCX files through Android's document picker and accepted both MIME types from other apps. It rendered PDF pages off the UI thread, but retained a bitmap and `ImageView` for every page. DOCX used one `TextView` containing the full extracted document.

First clean-ish debug build on 2026-07-15: **66.95 seconds**, **21,003-byte APK**. Generated output and all Gradle caches were under `C:\tmp\lawpdf-android`.

## Iteration 2: bounded and virtualized

- PDF pages now use a recycling `ListView`; only visible page rows own views.
- A memory-bounded `LruCache` retains rendered pages, capped at the smaller of 64 MiB or one eighth of the app heap.
- Render width is bucketed in 128-pixel increments and capped at 2048 pixels, avoiding duplicate work for tiny viewport changes and oversized phone bitmaps.
- A single renderer thread respects `PdfRenderer`'s access constraints. Visible pages have priority and each completed page prefetches its immediate neighbors.
- Document generations cancel stale results and clear queued work when another file opens.
- Android memory-pressure callbacks trim or clear the page cache.
- DOCX package/XML parsing remains streaming and size-bounded. Text is split into recycling paragraph rows, with very large paragraphs chunked at 8,000 UTF-16 code units.

Final verification on 2026-07-15:

- Debug APK: **36,117 bytes**, SHA-256 `44BE740388CA96DDD1469C62AA25FA32172AC2C2AAD1AF33EACC664CE63B1329`.
- Minified unsigned release APK: **16,044 bytes**, SHA-256 `A76F5B146A069CBD973504AF3861A8B7E871D44BCCB34D73A5A8B026553CF609`.
- Android DOCX tests: **2 passed, 0 failed**.
- Android lint: **0 errors, 0 warnings**.
- Windows DOCX regression tests: **2 passed, 0 failed**.

No Android device was connected for frame-time or heap profiling. The performance claim for this iteration is therefore based on the verified change in resource bounds: baseline PDF memory grew with every page, while the optimized path holds recycled visible rows plus an explicitly bounded cache of at most 64 MiB. Device profiling remains a separate release-validation step.

## Iteration 3: touch editing and portable export

- PDF rows now draw through a custom page surface that supports continuous vertical scrolling, 100-400% pinch zoom, and horizontal panning while preserving recycled rows.
- Ink, highlight, and signature strokes use normalized page coordinates, so marks stay aligned across zoom levels and during export.
- The collapsible tools strip adds Pan, Pen, Highlight, Sign, Eraser, four ink colors, Undo, Redo, Fit, and Save copy controls with accessibility labels and live status updates.
- Undo and redo retain reversible add/erase operations. A second finger cancels an uncommitted stroke before beginning a pinch, avoiding accidental dots.
- Annotated export is streamed one page at a time through a temporary cache file and then the Storage Access Framework destination. Rendering is capped at 2,560 pixels wide, and the original source is never modified.
- Android 15 system-bar/cutout handling keeps all top controls below intercepted system UI, with a conservative 32 dp fallback if the top inset is briefly zero.

Verification on a connected Pixel 9 Pro on 2026-07-15:

- The debug app installed successfully and opened a one-page PDF through an Android document-provider URI without an app exception.
- Android unit tests: **5 passed, 0 failed** (three annotation-history tests and two DOCX-reader tests).
- Android lint: **0 errors, 0 warnings**.
- Build output, Gradle caches, lint reports, test results, and APKs remained under `C:\tmp\lawpdf-android`.

The phone was securely locked during automated inspection, so final finger-gesture and document-picker interaction remain device-interactive validation rather than an automated claim.

## Iteration 4: stable zoom frames and reader-first chrome

- Pinch handling now uses Android's `ScaleGestureDetector`; double-tap zoom and tap-to-toggle chrome use `GestureDetector`.
- A zoom frame never clears the currently displayed page bitmap. The existing bitmap scales immediately, row geometry is coalesced to one update per animation frame, and the exact high-resolution bucket is requested after the gesture settles.
- Render completion swaps only matching visible rows. It no longer invalidates and rebinds the full page list, removing the white interval that caused the visible flash.
- If an exact render bucket is absent, the closest cached bitmap for that page remains visible as a temporary fallback.
- The expanded app bar can collapse to a compact reader row. Editing controls live in a separate horizontal palette and retain 48 dp touch targets and spoken descriptions.
- Default-PDF setup stays inside Android's user-consent model: LawPDF identifies the current resolver state, opens the system chooser when no default exists, and routes through app settings when another handler is already preferred.

Verification on 2026-07-15:

- Debug APK: **63,306 bytes**, SHA-256 `6B2AB46E6A8F32E979CAE824B3EB12E5114C41DB89AAD486F6592435A0C7A4CB`.
- Android unit tests: **11 passed, 0 failed**.
- Android lint: **0 errors, 0 warnings**.
- Debug packaging: **successful**.
- All generated output remained under `C:\tmp\lawpdf-android`.

Installation and finger-level validation are pending only because the previously connected Pixel disappeared from `adb devices` before this build was ready.
