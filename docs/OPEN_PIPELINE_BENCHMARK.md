# Measure PDF opening and first raster

The development build can time the same recovery snapshot, document opening,
annotation read, first CPU raster, and preparation of pixels for the UI used by the reader. It reports JSON to
standard output, including times for the recovery snapshot, metadata,
annotations, and the final revision check. It does not measure window creation,
GPU upload, or display.

The optimized opening path reads page dimensions through PDFium's indexed size
query, falling back to loading a page if that query fails or returns invalid
dimensions. This avoids loading page contents merely to measure them. A damaged
page can still fail later when it is rendered or its text is requested.

Choose a **new, non-existent** data directory outside the checkout. The command
creates it and stores a recovery snapshot and performance cache there, so it can
contain a copy of the input PDF. Keep it private when the PDF is private. The
input file is checked for changes and is not modified.

On Windows, from the repository root:

```powershell
$env:CARGO_TARGET_DIR = 'C:\tmp\lawpdf-target'
$env:LAWPDF_BENCH_DATA_DIR = 'C:\tmp\lawpdf-open-bench-run-001'
cargo run --locked --features devtools -- --bench-open-pipeline 'C:\path\to\document.pdf' 1.0
```

The final argument is an optional raster scale from 0.25 through 4.0 (default
1.0). Use a different new data directory for every cold application-cache run.
The operating system's file cache can still be warm; record the machine, file,
scale, cache condition, and multiple trials when comparing results. The test
does not establish real-window first-page latency by itself.

For an eligible large PDF, `read_only_preview_ready_ms` records when a temporary
first-page CPU raster and prepared pixels arrive while the verified recovery snapshot is still
being prepared. It can be null if a preview could not be produced or the full
open finished first. `first_page_ready_ms` remains the time to a prepared raster after the
document is fully opened and its revision checked. Neither includes GPU upload.
