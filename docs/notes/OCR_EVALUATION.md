# Local OCR Evaluation

Date: 2026-06-03

## Current Runtime Baseline

LawPDF uses PDFium native text first. Local OCR renders each page to PNG and
runs Tesseract. The app previously forced Tesseract `--psm 6`, which assumes a
single uniform block of text. The current local setting is now `--psm 4`, which
assumes a single column with variable text sizes and performed better on the
local benchmark pages.

The OpenRouter OCR path is separate and not a local engine.

## Local Engines Checked

Installed locally:

- `tesseract.exe`: available on PATH, version `5.5.0.20241111`.
- `surya_ocr.exe`, `surya_layout.exe`: available on PATH. It imported again
  after restoring `transformers` to the Surya-compatible `4.x` series.
- `paddleocr`: installed as a Python package, version `3.6.0`.
- `PP-DocLayoutV3`: cached locally under `C:\Users\yonat\.paddlex\official_models\PP-DocLayoutV3`.
- Chandra: installed in an isolated venv at
  `C:\tmp\lawpdf-chandra-venv` and callable with
  `CHANDRA_EXE=C:\tmp\lawpdf-chandra-venv\Scripts\chandra.exe`.

Not installed:

- `easyocr`
- `doctr`

## Benchmark Harness

Script:

```powershell
python tools\ocr_engine_benchmark.py --help
```

The harness renders selected PDF pages with PyMuPDF, runs OCR/layout engines,
records runtime, text length, detected box counts, average confidence when
available, and rough similarity to native PDF text when selectable text exists.

Primary outputs:

- `pages.json`
- `results.json`
- `results.jsonl`
- `summary.json`

The harness can also synthesize degraded scan-like page images:

```powershell
--noise-profile clean,scan-light,scan-heavy
```

It can benchmark exact page samples emitted by the PP-DocLayout sampler:

```powershell
--page-specs-jsonl C:\tmp\lawpdf-ocr-footnote-sample.jsonl
```

It can benchmark Chandra through an isolated executable:

```powershell
$env:CHANDRA_EXE = "C:\tmp\lawpdf-chandra-venv\Scripts\chandra.exe"
python tools\ocr_engine_benchmark.py --pdf sample.pdf --pages 0 --engine chandra-hf
```

## Local Smoke Results

Course/survey-like page:

```powershell
python tools\ocr_engine_benchmark.py `
  --pdf "to-evaluate\liquid-quality-risk-v1\pdfs\079_q1_law_review_article_2021.PDF" `
  --pages 0 `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-smoke `
  --engine tesseract-psm3 `
  --engine tesseract-psm4 `
  --engine tesseract-psm6 `
  --engine surya-ocr
```

Summary:

- `tesseract-psm4`: `1.807s`, native similarity `0.9266`
- `tesseract-psm3`: `2.014s`, native similarity `0.9263`
- `tesseract-psm6`: `1.543s`, native similarity `0.8694`
- `surya-ocr`: `16.420s`, native similarity `0.5797`

Article/application-packet pages:

```powershell
python tools\ocr_engine_benchmark.py `
  --pdf "to-evaluate\liquid-quality-risk-v69\pdfs\080_q3_law_review_article_Christian Johnson.pdf" `
  --pages 0,1 `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-lawreview `
  --engine tesseract-psm3 `
  --engine tesseract-psm4 `
  --engine tesseract-psm6 `
  --engine surya-ocr `
  --engine surya-layout `
  --engine paddle-layout
```

Summary:

- `tesseract-psm4`: `0.781s/page`, average native similarity `0.4884`
- `tesseract-psm3`: `0.799s/page`, average native similarity `0.4885`
- `tesseract-psm6`: `0.661s/page`, average native similarity `0.0541`
- `surya-ocr`: `12.941s/page`, average native similarity `0.4505`
- `paddle-layout`: `10.539s/page`, layout boxes detected
- `surya-layout`: `23.545s/page`, layout boxes detected

Harvard Law Review article pages:

```powershell
python tools\ocr_engine_benchmark.py `
  --pdf "to-evaluate\profile-active-20260529-171019\pdfs\011_law_review_article_current-other_138-Harv.-L.-Rev.-1609-1.pdf" `
  --pages 0,1 `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-harvard-fast `
  --engine tesseract-psm3 `
  --engine tesseract-psm4 `
  --engine tesseract-psm6 `
  --engine paddle-layout
```

Summary:

- `tesseract-psm4`: `1.120s/page`, same word count as `psm3`
- `tesseract-psm3`: `1.149s/page`
- `tesseract-psm6`: `1.027s/page`
- `paddle-layout`: `7.327s/page`

Important layout observation: Paddle PP-DocLayoutV3 detected three `footnote`
regions on page 2 of the Harvard sample:

```text
footnote:0.861
footnote:0.843
footnote:0.926
```

Noisy Harvard Law Review page:

```powershell
python tools\ocr_engine_benchmark.py `
  --pdf "to-evaluate\profile-active-20260529-171019\pdfs\011_law_review_article_current-other_138-Harv.-L.-Rev.-1609-1.pdf" `
  --pages 1 `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-harvard-noisy `
  --noise-profile clean,scan-light,scan-heavy `
  --engine tesseract-psm3 `
  --engine tesseract-psm4 `
  --engine tesseract-psm6 `
  --engine paddle-layout
```

Summary:

- `tesseract-psm4`: `1.105s/page`, average native similarity `0.7213`
- `tesseract-psm3`: `1.132s/page`, average native similarity `0.7213`
- `tesseract-psm6`: `1.032s/page`, average native similarity `0.7210`
- `paddle-layout`: `6.946s/page`, layout boxes detected

The clean-render similarity on this page is artificially low because the PDF
native text order is not a perfect ground truth. The same page under the heavy
scan profile reached about `0.986` native similarity with Tesseract `psm4`.

Noisy OCR-stack comparison on the same Harvard page:

```powershell
python tools\ocr_engine_benchmark.py `
  --pdf "to-evaluate\profile-active-20260529-171019\pdfs\011_law_review_article_current-other_138-Harv.-L.-Rev.-1609-1.pdf" `
  --pages 1 `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-harvard-ocrstack `
  --noise-profile scan-heavy `
  --engine tesseract-psm4 `
  --engine paddle-ocr `
  --engine surya-ocr `
  --engine paddle-layout `
  --engine surya-layout
```

Summary:

- `tesseract-psm4`: `1.113s`, native similarity `0.9859`
- `surya-ocr`: `11.439s`, native similarity `0.9625`
- `paddle-layout`: `4.467s`, layout boxes detected
- `surya-layout`: `6.433s`, layout boxes detected
- `paddle-ocr`: failed locally with a Paddle oneDNN runtime error after model download

Footnote-heavy law-review pages selected from the existing layout-role training
examples:

```powershell
python tools\pp_doclayout_deployment.py sample `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --output C:\tmp\lawpdf-ocr-footnote-sample.jsonl `
  --limit 5 `
  --strict-law-review `
  --min-page-footnotes 8

python tools\ocr_engine_benchmark.py `
  --page-specs-jsonl C:\tmp\lawpdf-ocr-footnote-sample-filtered.jsonl `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-footnote-pages `
  --noise-profile clean,scan-heavy `
  --engine tesseract-psm3 `
  --engine tesseract-psm4 `
  --engine tesseract-psm6 `
  --engine paddle-layout
```

The filtered sample used three pages with known footnote-line counts of `30`,
`31`, and `16`. Summary across clean and heavy-scan renders:

- `tesseract-psm4`: `1.221s/page`, average native similarity `0.9793`
- `tesseract-psm3`: `1.248s/page`, average native similarity `0.9793`
- `tesseract-psm6`: `1.152s/page`, average native similarity `0.9780`
- `paddle-layout`: `5.509s/page`, `24` `footnote` regions detected across
  the six rendered page/profile cases

Single-page expensive fallback comparison on a degraded footnote-heavy page:

- `tesseract-psm4`: `1.272s`, native similarity `0.9621`
- `surya-ocr`: `11.565s`, native similarity `0.9929`
- `paddle-layout`: `9.839s`, detected `1` `footnote` region
- `surya-layout`: `7.734s`, detected `3` `footnote` regions

Chandra HF smoke test on one clean footnote-heavy page:

```powershell
$env:CHANDRA_EXE = "C:\tmp\lawpdf-chandra-venv\Scripts\chandra.exe"
python tools\ocr_engine_benchmark.py `
  --page-specs-jsonl C:\tmp\lawpdf-ocr-footnote-sample-one.jsonl `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-chandra-one `
  --noise-profile clean `
  --engine tesseract-psm4 `
  --engine chandra-hf `
  --timeout 300
```

Summary:

- `tesseract-psm4`: `1.237s`, native similarity `0.9623`
- `chandra-hf`: timed out after `300s` on the same single rendered page

Chandra dependency note: `chandra-ocr[hf]` currently requires
`transformers>=5.2` and `torch>=2.8`, which conflicts with the installed Surya
stack (`transformers<5`). The benchmark therefore supports `CHANDRA_EXE` so
Chandra can live in a separate venv. The Chandra venv created here installed a
CPU-only PyTorch build on Windows, and that was not performant enough for the
default local OCR path.

Fine-tuning note: the installed `chandra-ocr 0.2.0` package exposes inference
code for the HuggingFace and vLLM paths, but no local training, LoRA, or
fine-tuning entry point was present in the package. Fine-tuning would require a
separate model-training workflow and GPU environment; it should wait until a
GPU/vLLM Chandra baseline beats the cheaper local ensemble on LawPDF pages.

## Recommendation

Use a tiered local strategy:

1. Keep PDFium native text as the first choice when native text is rich enough.
2. Change local Tesseract fallback from `--psm 6` to `--psm 4`.
3. Add Paddle PP-DocLayoutV3 as a layout oracle for scanned or OCR-backed pages:
   use it to detect `footnote`, `header`, `footer`, `number`, `table`, and body
   regions, then route OCR text/boxes through the existing layout-role logic.
4. Do not use Paddle OCR text recognition in the current local environment. The
   OCR model path failed locally; Paddle's layout detector is the useful part.
5. Do not run Surya by default. It is useful as an expensive fallback or audit
   engine, and on one degraded footnote-heavy page it beat Tesseract text
   similarity, but it was about `9x` slower than Tesseract text recognition.
6. Keep Chandra as an optional experiment only. The adapter is implemented in
   the benchmark harness, but local HF inference on the current CPU-only Windows
   environment timed out after `300s` on one page. It is not a viable default
   until tested through a GPU-capable environment or vLLM server.

This avoids making every page expensive. The normal path stays PDFium/Tesseract,
and Paddle layout is paid only when we need layout-sensitive OCR or when the
native-text/Liquid confidence says the page is scanned, sparse, or structurally
suspect.

## Primary Documentation Checked

- Tesseract command line documentation:
  https://github.com/tesseract-ocr/tessdoc/blob/main/Command-Line-Usage.md
- PaddleOCR layout detection documentation:
  https://www.paddleocr.ai/main/en/version3.x/module_usage/layout_detection.html
- PaddleOCR PP-StructureV3 documentation:
  https://www.paddleocr.ai/main/en/version3.x/pipeline_usage/PP-StructureV3.html
- Surya README:
  https://github.com/datalab-to/surya
