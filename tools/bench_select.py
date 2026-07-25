#!/usr/bin/env python3
"""Select a stratified law review benchmark that shares no document with training.

Two filters matter and both were learned the hard way.

*Exclude the training corpus.* Two of the five articles in the original
agentic-review corpus turn out to be in it, which made a retrained model look
like it improved in-sample while held-out quality halved.

*Require a footnote apparatus.* Journal archives are not all articles. An
`oregon_law_review` folder holds the alumni magazine and program-evaluation
reports; a `south_carolina_law_review` folder holds book-review round-ups.
Those documents drag mean recall down by 18 points while measuring something
the reconstruction pipeline was never aimed at. A document qualifies only if it
carries at least `--min-note-heads` small-font lines that open like a numbered
footnote.

Usage:
    python tools/bench_select.py --index journal-index.json --train train.jsonl \\
        --out bench-pdfs --manifest bench-manifest.json
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import shutil
import statistics
import sys
from pathlib import Path

NOTE_HEAD = re.compile(r"^\d{1,3}[.\s]")


def note_head_count(doc, max_pages: int = 25) -> int:
    """Small-font lines that open like `12.` or `12 ` — a footnote apparatus."""
    heads = 0
    for index in range(min(doc.page_count, max_pages)):
        page = doc[index]
        sizes = [
            round(span["size"], 2)
            for block in page.get_text("dict")["blocks"]
            for line in block.get("lines", [])
            for span in line["spans"]
            if span["text"].strip()
        ]
        if not sizes:
            continue
        median = statistics.median(sizes)
        for block in page.get_text("dict")["blocks"]:
            for line in block.get("lines", []):
                text = "".join(s["text"] for s in line["spans"]).strip()
                spans = [round(s["size"], 2) for s in line["spans"] if s["text"].strip()]
                if spans and max(spans) < median - 0.4 and NOTE_HEAD.match(text):
                    heads += 1
    return heads


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--index", type=Path, required=True, help="journal -> [pdf paths] JSON")
    parser.add_argument("--train", type=Path, required=True, help="training JSONL, for exclusion")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--per-journal", type=int, default=5)
    parser.add_argument("--min-note-heads", type=int, default=20)
    parser.add_argument("--min-pages", type=int, default=8)
    parser.add_argument("--max-pages", type=int, default=120)
    parser.add_argument("--seed", type=int, default=20260725)
    args = parser.parse_args(argv)

    import fitz

    train = set()
    with args.train.open(encoding="utf-8") as handle:
        for line in handle:
            train.add(os.path.basename(json.loads(line)["doc"]).lower())
    print(f"excluding {len(train)} training documents", flush=True)

    index = json.loads(args.index.read_text(encoding="utf-8"))
    shutil.rmtree(args.out, ignore_errors=True)
    args.out.mkdir(parents=True, exist_ok=True)

    rng = random.Random(args.seed)
    manifest = []
    for journal, pdfs in sorted(index.items()):
        clean = [p for p in pdfs if os.path.basename(p).lower() not in train]
        rng.shuffle(clean)
        taken = 0
        for path in clean:
            if taken >= args.per_journal:
                break
            try:
                doc = fitz.open(path)
            except Exception:
                continue
            if not (args.min_pages <= doc.page_count <= args.max_pages):
                continue
            sample = "".join(doc[i].get_text() for i in range(min(6, doc.page_count)))
            if len(sample) < 6000:            # scanned without a text layer
                continue
            heads = note_head_count(doc)
            if heads < args.min_note_heads:   # not an article with footnotes
                continue
            stem = "b{:02d}-{}-{}".format(
                len(manifest) + 1,
                journal.split("_")[0][:9],
                re.sub(r"[^A-Za-z0-9]+", "-", Path(path).stem).strip("-").lower()[:26],
            )
            shutil.copy(path, args.out / f"{stem}.pdf")
            manifest.append(
                {
                    "stem": stem,
                    "journal": journal,
                    "source": path,
                    "pages": doc.page_count,
                    "note_heads": heads,
                }
            )
            taken += 1
            print(f"  {stem}  ({doc.page_count}p, {heads} note heads)", flush=True)
        print(f"{journal}: {taken} selected from {len(clean)} clean candidates", flush=True)

    args.manifest.write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    print(f"\nselected {len(manifest)} documents -> {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
