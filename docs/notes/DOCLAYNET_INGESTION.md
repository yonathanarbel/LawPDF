# DocLayNet Ingestion Contract

Preferred local root: `C:\tmp\doclaynet-v1.1`.

Ask the download/prep agent to create either an embedded page file or the
normalized three-table output that `tools/prepare_doclaynet_v1_1.py` emits.
The normalized output is the path used in the current pilot:

- `pages.jsonl`: one page per row, with embedded `regions` and `pdf_cells`.
- `processed/pages.jsonl`: one page metadata row per page.
- `processed/regions.jsonl`: one layout annotation row per DocLayNet region.
- `processed/cells.jsonl`: one extracted PDF text-cell row per cell.
- Page image files on disk, referenced by path when available. Do not embed images as base64.
- `manifest.json`: source dataset revision, split counts, category counts, file hashes, and prep command.
- Optional visual QA overlays for at least 20 mixed pages, including footnote-heavy and header/footer-heavy pages.

Each `pages.jsonl` row should include:

```json
{
  "page_id": "stable page id",
  "doc_id": "stable document id",
  "split": "train|val|test",
  "category": "laws_and_regulations|manuals|patents|scientific_articles|financial_reports|government_tenders",
  "page_index": 0,
  "page_width": 1025,
  "page_height": 1025,
  "image_path": "C:\\tmp\\doclaynet-v1.1\\images\\...",
  "regions": [
    {"id": "r1", "label": "Text", "bbox": [x, y, width, height]}
  ],
  "pdf_cells": [
    {"id": "c1", "text": "extracted PDF cell text", "bbox": [x, y, width, height]}
  ]
}
```

Use top-left origin coordinates. `bbox` may be absolute pixels or normalized 0-1 values; `tools/doclaynet_ingest.py` supports either. If the prep uses bottom-left PDF coordinates, say so and run with `--input-origin bottom-left`.

DocLayNet labels are mapped into LawPDF roles as:

- `Text` -> `body`
- `Footnote` -> `footnote`
- `Title` -> `title`
- `Section-header` -> `heading`
- `Page-header`, `Page-footer` -> `header_footer`
- `List-item` -> `list_item`
- `Table` -> `table`
- `Caption` -> `caption`
- `Picture`, `Formula` -> `visual`

Ingest command for the embedded-page schema:

```powershell
python tools\doclaynet_ingest.py `
  --pages-jsonl C:\tmp\doclaynet-v1.1\pages.jsonl `
  --examples-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-examples.json `
  --labels-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-labels.json `
  --assignments-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-cell-assignments.jsonl `
  --report-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-ingest-report.json
```

Ingest command for the normalized tables emitted by
`tools\prepare_doclaynet_v1_1.py`:

```powershell
python tools\doclaynet_ingest.py `
  --pages-table-jsonl C:\tmp\doclaynet-v1.1\processed\pages.jsonl `
  --regions-jsonl C:\tmp\doclaynet-v1.1\processed\regions.jsonl `
  --cells-jsonl C:\tmp\doclaynet-v1.1\processed\cells.jsonl `
  --bbox-format xyxy `
  --examples-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-examples.json `
  --labels-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-labels.json `
  --assignments-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-cell-assignments.jsonl `
  --report-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-ingest-report.json `
  --source doclaynet_v1_1 `
  --weight 1.0
```

For the full DocLayNet-v1.1 prepared tables, prefer the streaming prepared-cell
path. The full `processed/cells.jsonl` is large enough that the batch converter
will materialize too much data in memory. The streaming path uses the prepared
cell `matched_region_category` assignments directly, keeps all uncapped roles,
and applies deterministic per-role caps for high-volume roles:

```powershell
python tools\doclaynet_ingest.py `
  --stream-prepared-cells `
  --pages-table-jsonl C:\tmp\doclaynet-v1.1\processed\pages.jsonl `
  --cells-jsonl C:\tmp\doclaynet-v1.1\processed\cells.jsonl `
  --bbox-format xyxy `
  --examples-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-stream-balanced-fullroles-v1-examples.json `
  --labels-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-stream-balanced-fullroles-v1-labels.json `
  --assignments-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-stream-balanced-fullroles-v1-assignments.jsonl `
  --report-output C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-stream-balanced-fullroles-v1-report.json `
  --source doclaynet_v1_1_stream_balanced `
  --weight 1.0 `
  --role-cap body=80000 `
  --role-cap table=80000 `
  --role-cap visual=80000 `
  --role-cap header_footer=60000 `
  --role-cap heading=60000 `
  --role-cap list_item=60000 `
  --role-cap caption=40000 `
  --sample-seed 42
```

The main full-role NB scheme does not include `visual`. For that model family,
drop or remap `visual` after streaming; keep the full-role stream for
`liquid_core` experiments where `visual` is valid.

The most important output for our next training run is `lawpdf-doclaynet-examples.json`: it contains generated `LayoutLine` rows from `pdf_cells`, with matching labels and geometry features. Do not merge these labels into the law-review label file until we inspect the ingest report and decide source weights.

The current deployed full-role NB model supports `table`, `caption`, and
`list_item`, but not `visual`. Drop or map `visual` rows before a full-role NB
candidate run, or train them only under the `liquid_core` role scheme where
`visual` is valid.

To combine DocLayNet with the current LawPDF cache after QA:

```powershell
python tools\layout_merge_examples.py `
  --examples-input C:\tmp\lawpdf-layout-role-examples-v5-unseen.json `
  --examples-input C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-examples.json `
  --output C:\tmp\lawpdf-layout-role-examples-v6-doclaynet-merged.json `
  --dedupe
```

Then train with both label sources, choosing DocLayNet weight explicitly:

```powershell
python tools\layout_role_training.py `
  --examples-input C:\tmp\lawpdf-layout-role-examples-v6-doclaynet-merged.json `
  --gold-labels C:\tmp\lawpdf-latest-labels-v5-unseen-catboost-highconf-train.json `
  --gold-labels C:\tmp\doclaynet-v1.1\lawpdf-doclaynet-labels.json `
  --gold-label-source-scale doclaynet_v1_1=0.5
```
