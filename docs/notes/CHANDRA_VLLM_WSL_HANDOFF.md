# Chandra vLLM WSL Handoff

Date: 2026-06-03

This is a handoff for continuing LawPDF OCR work from inside WSL with a new
Codex agent. The immediate goal is to get `datalab-to/chandra` / Chandra OCR
running through vLLM on the local RTX 3090, then benchmark it against the current
Tesseract path on law-review pages with dense footnotes.

## Primary Goal

1. Start a working local vLLM OpenAI-compatible server for
   `datalab-to/chandra-ocr-2`.
2. Run the repo benchmark engine `chandra-vllm` against the existing
   footnote-heavy sample page.
3. Compare output quality and latency against `tesseract-psm4`.
4. If Chandra works, decide whether to wire it into LawPDF as an optional OCR
   backend and then investigate fine-tuning.

Do not spend more time on the Chandra HF path unless needed for debugging. It
timed out even with CUDA and is not the desired route.

## Important Directories

Windows repo:

```text
C:\Users\yonat\Box\Gmailer\lawpdf
```

Same repo from WSL:

```text
/mnt/c/Users/yonat/Box/Gmailer/lawpdf
```

Existing WSL vLLM environment:

```text
/home/arbel/lawpdf-chandra-vllm
```

Existing Windows Chandra CLI virtualenv:

```text
C:\tmp\lawpdf-chandra-venv
```

Windows Chandra executable:

```text
C:\tmp\lawpdf-chandra-venv\Scripts\chandra.exe
```

Useful benchmark input:

```text
C:\tmp\lawpdf-ocr-footnote-sample-one.jsonl
```

Same benchmark input from WSL:

```text
/mnt/c/tmp/lawpdf-ocr-footnote-sample-one.jsonl
```

Existing rendered clean page image:

```text
C:\tmp\lawpdf-ocr-benchmark-chandra-gpu-one\images\loyola_chicago_vol20_iss2_Article_15-p0004-clean.png
```

Same image from WSL:

```text
/mnt/c/tmp/lawpdf-ocr-benchmark-chandra-gpu-one/images/loyola_chicago_vol20_iss2_Article_15-p0004-clean.png
```

Recommended new output directory:

```text
C:\tmp\lawpdf-ocr-benchmark-chandra-vllm-one
```

## Known Environment State

WSL:

- Default WSL distro is Ubuntu under WSL2.
- WSL user home observed as `/home/arbel`.
- `nvidia-smi` works inside WSL and sees `NVIDIA GeForce RTX 3090`.
- GPU memory was mostly free after cleanup: about 22.7 GB free out of 24 GB.
- WSL venv `/home/arbel/lawpdf-chandra-vllm` exists.
- That venv had:
  - `vllm 0.17.0`
  - `torch 2.10.0+cu128`
  - CUDA available

Windows:

- Docker is not installed/on PATH. Chandra's packaged `chandra_vllm` helper is
  a Docker launcher, so it is not useful on this Windows setup unless Docker is
  installed later.
- The Windows Chandra CLI venv exists and can run `chandra.exe --help`.
- The Windows Chandra CLI with `--method hf` timed out on one page, both CPU and
  CUDA.

## Repo State To Know

The repo has many existing modified/untracked files and training artifacts. Do
not clean or reset the worktree unless the user explicitly asks.

Relevant files:

```text
tools/ocr_engine_benchmark.py
tools/setup_chandra_venv.ps1
OCR_EVALUATION.md
README.md
src/ocr.rs
src/liquid/normalization.rs
training-data/
```

Already implemented before this handoff:

- `src/ocr.rs` uses local Tesseract PSM 4.
- `src/liquid/normalization.rs` has a repair for repeated-marker law-review
  marginalia footnote lines, with a focused test.
- `tools/ocr_engine_benchmark.py` supports `chandra-hf` and `chandra-vllm`.
- The release package was rebuilt on Windows:
  - `target\release\lawpdf.exe`
  - `dist\LawPDF-portable\lawpdf.exe`
  - `dist\LawPDF-windows-portable-x64.zip`

Earlier verification:

```powershell
cargo test profile_policy_merges_repeated_marker_lines_from_single_law_review_note
cargo test ocr
python -m py_compile tools\ocr_engine_benchmark.py
cargo build --release
```

## Benchmark Results So Far

Single clean footnote-heavy page:

- `tesseract-psm4`: about 1.16 seconds, text similarity about 0.9623.
- `chandra-hf` on Windows CPU: timed out after 300 seconds.
- `chandra-hf` on Windows CUDA: timed out after 600 seconds.

Conclusion: do not evaluate Chandra through the HF route for normal workflow.
Use vLLM.

## First WSL Commands To Run

Enter WSL and move to the repo:

```bash
cd /mnt/c/Users/yonat/Box/Gmailer/lawpdf
```

Confirm GPU and venv:

```bash
nvidia-smi
source ~/lawpdf-chandra-vllm/bin/activate
python - <<'PY'
import torch, vllm
print("vllm", vllm.__version__)
print("torch", torch.__version__)
print("cuda", torch.cuda.is_available())
print(torch.cuda.get_device_name(0) if torch.cuda.is_available() else "no cuda")
PY
```

Kill any stale vLLM process:

```bash
pgrep -af 'vllm serve datalab-to/chandra-ocr-2' || true
pkill -TERM -f 'vllm serve datalab-to/chandra-ocr-2' || true
sleep 3
pgrep -af 'vllm serve datalab-to/chandra-ocr-2' || true
```

Start vLLM with conservative RTX 3090 settings:

```bash
cat > ~/start_chandra_vllm.sh <<'SH'
#!/usr/bin/env bash
set -euo pipefail
source ~/lawpdf-chandra-vllm/bin/activate
exec vllm serve datalab-to/chandra-ocr-2 \
  --host 0.0.0.0 \
  --port 8000 \
  --served-model-name chandra \
  --dtype float16 \
  --max-model-len 8192 \
  --max-num-seqs 4 \
  --max-num-batched-tokens 1024 \
  --gpu-memory-utilization 0.80 \
  --enforce-eager
SH
chmod +x ~/start_chandra_vllm.sh
nohup ~/start_chandra_vllm.sh > ~/lawpdf-chandra-vllm-run.log 2>&1 &
echo $! > ~/lawpdf-chandra-vllm-run.pid
tail -f ~/lawpdf-chandra-vllm-run.log
```

In another WSL shell, poll readiness:

```bash
curl -s http://127.0.0.1:8000/v1/models | jq .
```

If `jq` is not installed:

```bash
curl -s http://127.0.0.1:8000/v1/models
```

## If vLLM Hangs During Model Load

Observed prior behavior: vLLM resolved the Chandra architecture and started
loading, then held GPU memory for a long time without serving `/v1/models`.

Try these one at a time:

1. Remove `--enforce-eager`.
2. Change `--dtype float16` to `--dtype bfloat16`.
3. Increase memory utilization:

```bash
--gpu-memory-utilization 0.90
```

4. Increase model length after it works:

```bash
--max-model-len 12000
```

5. Check whether the process is alive and whether logs moved:

```bash
ps -fp "$(cat ~/lawpdf-chandra-vllm-run.pid)" || true
tail -200 ~/lawpdf-chandra-vllm-run.log
nvidia-smi
```

The Chandra package's Docker launcher would use these approximate settings for
a 3090:

```text
--max-num-batched-tokens 2048
--max-num-seqs 16
--dtype bfloat16
--max-model-len 18000
--gpu-memory-utilization 0.85
--enable-prefix-caching
--served-model-name chandra
```

Those are more aggressive than the recommended first boot settings above.

## Benchmark Once vLLM Is Serving

From Windows PowerShell, with the vLLM server running in WSL:

```powershell
cd C:\Users\yonat\Box\Gmailer\lawpdf
$env:CHANDRA_EXE = "C:\tmp\lawpdf-chandra-venv\Scripts\chandra.exe"
$env:VLLM_API_BASE = "http://127.0.0.1:8000/v1"
$env:VLLM_MODEL_NAME = "chandra"
python tools\ocr_engine_benchmark.py `
  --page-specs-jsonl C:\tmp\lawpdf-ocr-footnote-sample-one.jsonl `
  --output-dir C:\tmp\lawpdf-ocr-benchmark-chandra-vllm-one `
  --noise-profile clean `
  --engine tesseract-psm4 `
  --engine chandra-vllm `
  --timeout 600
```

From WSL, if you prefer to run the benchmark there, first ensure the repo Python
dependencies are installed in the active environment. Then use:

```bash
cd /mnt/c/Users/yonat/Box/Gmailer/lawpdf
export VLLM_API_BASE=http://127.0.0.1:8000/v1
export VLLM_MODEL_NAME=chandra
python tools/ocr_engine_benchmark.py \
  --page-specs-jsonl /mnt/c/tmp/lawpdf-ocr-footnote-sample-one.jsonl \
  --output-dir /mnt/c/tmp/lawpdf-ocr-benchmark-chandra-vllm-one \
  --noise-profile clean \
  --engine tesseract-psm4 \
  --engine chandra-vllm \
  --timeout 600
```

Note: the benchmark currently calls the Chandra CLI for `chandra-vllm`. That
means it may still use the Windows Chandra executable when run from PowerShell.
If running fully inside WSL, install `chandra-ocr` in the WSL vLLM venv or point
`CHANDRA_EXE` at a Linux `chandra` executable.

## Small Benchmark Patch Worth Doing

`tools/ocr_engine_benchmark.py` currently does not pass `--max-output-tokens` to
the Chandra CLI. Chandra's default output token limit is high and can make test
runs too slow. Add a small env-var-controlled cap before running broad tests.

Suggested behavior:

```text
CHANDRA_MAX_OUTPUT_TOKENS defaults to 2048
benchmark_chandra adds:
  --max-output-tokens <value>
  --batch-size 1
```

Keep this scoped to `tools/ocr_engine_benchmark.py`.

## Fine-Tuning Goal

Fine-tuning is a second-stage goal after inference is working.

Questions to answer after vLLM inference works:

1. Does Chandra's repo expose training scripts or only inference?
2. Does `datalab-to/chandra-ocr-2` require Qwen 3.5 support beyond current
   public trainer support?
3. Can the existing LawPDF training data be converted into Chandra's expected
   image-plus-markdown or image-plus-text format?
4. Is fine-tuning necessary, or does vLLM Chandra already fix the footnote and
   marginalia failure modes enough to ensemble with Tesseract?

Relevant training data lives under:

```text
training-data/
training-data/layout-role-core/
training-data/doclaynet/
training-data/ppdoclayout/
training-data/word-groundtruth/
```

Yes, DocLayNet-derived training materials were used for layout-role/model work.
They are in:

```text
training-data/doclaynet/
training-data/layout-role-core/lawpdf-layout-role-examples-v6-doclaynet-stream-balanced-mainroles-20260603.json
```

## User-Facing Product Goal

The user is trying to fix law-review PDFs where footnotes and marginalia are
handled badly. Current known failure modes:

- Every line of a footnote can become a separate marginalia item.
- We want lines belonging to the same footnote number combined.
- OCR should be more layout-sensitive than plain Tesseract if practical.

The acceptance bar is not just "Chandra runs"; it must improve law-review
footnote/marginalia extraction enough to justify its GPU cost.
