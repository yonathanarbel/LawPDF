#!/usr/bin/env python3
"""Extract per-line label provenance from a layout-role examples file.

85% of the LM2 training corpus is `silver` — labels the pipeline produced for
itself — and only a minority come from a teacher. Whether that hurts is
testable: emit the provenance keyed the same way `--dump-lm2-training` keys its
rows, then train with and without a weighting that trusts teachers more.

Writes JSONL: {"k": "<doc>|<page>|<line>", "s": "<label_source>", "w": <weight>}

Usage:
    python tools/lm2_label_sources.py --examples <examples.json> --out sources.jsonl
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--examples", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)

    print(f"loading {args.examples} (large; expect a minute) ...", flush=True)
    with args.examples.open(encoding="utf-8") as handle:
        payload = json.load(handle)
    lines = payload["lines"]
    print(f"  {len(lines)} lines")

    sources: Counter[str] = Counter()
    with args.out.open("w", encoding="utf-8") as out:
        for line in lines:
            source = line.get("label_source") or "unknown"
            sources[source] += 1
            record = {
                "k": f"{line['path']}|{line['page_index']}|{line['line_index']}",
                "s": source,
                "w": float(line.get("train_weight", 1.0)),
            }
            out.write(json.dumps(record) + "\n")

    print(f"wrote {args.out}")
    for source, count in sources.most_common():
        print(f"  {count:>7}  {source}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
