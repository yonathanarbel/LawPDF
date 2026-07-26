#!/usr/bin/env python3
"""Compare two benchmark sweeps without being fooled by coverage changes.

A raw defect count cannot adjudicate a change that alters how many documents
emit footnotes at all. The footnote checks only inspect documents that emit
numbered definitions, so a document emitting none escapes them and scores as
clean while actually being broken. Turning footnotes on for such a document
makes pre-existing damage visible and the total goes *up*.

So this reports three things separately:

* **Coverage** — how many documents got linked footnotes, and how many.
* **Fidelity** — of the definitions emitted, how many are referenced from the
  body. This is what tells you the new footnotes are real rather than orphans.
* **Like-for-like defects** — restricted to documents that emit footnotes in
  *both* runs, which is the only subset where the checks are active on both
  sides and a rise means genuine regression.

Usage:
    python tools/bench_compare.py --before before.json --after after.json
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path


def load(path: Path) -> dict[str, dict]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    return {
        Path(doc["path"]).stem: doc for doc in payload["documents"]
    }


def emits(doc: dict) -> bool:
    return bool(doc["stats"].get("footnote_definitions"))


def critical(doc: dict) -> Counter:
    return Counter(
        f["kind"] for f in doc["defects"] if f.get("severity") == "critical"
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument(
        "--max-regression",
        type=int,
        default=0,
        help="like-for-like defect increase tolerated before exiting non-zero",
    )
    args = parser.parse_args(argv)

    before, after = load(args.before), load(args.after)
    shared = sorted(set(before) & set(after))
    if not shared:
        raise SystemExit("the two runs share no documents")

    def totals(runs: dict[str, dict]) -> tuple[int, int, int, float]:
        docs = sum(1 for k in shared if emits(runs[k]))
        defs = sum(runs[k]["stats"].get("footnote_definitions") or 0 for k in shared)
        refs = sum(runs[k]["stats"].get("footnote_references") or 0 for k in shared)
        recalls = [
            runs[k]["stats"]["source_recall"]
            for k in shared
            if "source_recall" in runs[k]["stats"]
        ]
        return docs, defs, refs, (sum(recalls) / len(recalls) if recalls else 0.0)

    b_docs, b_defs, b_refs, b_recall = totals(before)
    a_docs, a_defs, a_refs, a_recall = totals(after)

    print(f"documents compared: {len(shared)}\n")
    print("COVERAGE")
    print(f"  documents with linked footnotes  {b_docs:>6} -> {a_docs:<6} ({a_docs - b_docs:+d})")
    print(f"  footnote definitions             {b_defs:>6} -> {a_defs:<6} ({a_defs - b_defs:+d})")
    print("\nFIDELITY  (definitions referenced from the body; should stay ~100%)")
    print(f"  before {100 * b_refs / max(1, b_defs):5.1f}%    after {100 * a_refs / max(1, a_defs):5.1f}%")
    print("\nCONTENT")
    print(f"  mean source recall               {b_recall:6.3f} -> {a_recall:.3f} ({a_recall - b_recall:+.3f})")

    # Like-for-like: only documents whose footnote checks were active in both.
    common = [k for k in shared if emits(before[k]) and emits(after[k])]
    b_common, a_common = Counter(), Counter()
    for key in common:
        b_common += critical(before[key])
        a_common += critical(after[key])
    b_total, a_total = sum(b_common.values()), sum(a_common.values())

    print(f"\nLIKE-FOR-LIKE DEFECTS  ({len(common)} documents emitting footnotes in both runs)")
    print(f"  critical defects                 {b_total:>6} -> {a_total:<6} ({a_total - b_total:+d})")
    changed = {k: a_common[k] - b_common[k] for k in set(b_common) | set(a_common)}
    for kind, delta in sorted(changed.items(), key=lambda kv: -abs(kv[1])):
        if delta:
            print(f"    {kind:<38} {b_common[kind]:>5} -> {a_common[kind]:<5} {delta:+d}")

    newly = [k for k in shared if not emits(before[k]) and emits(after[k])]
    if newly:
        defs = sum(after[k]["stats"].get("footnote_definitions") or 0 for k in newly)
        refs = sum(after[k]["stats"].get("footnote_references") or 0 for k in newly)
        print(f"\nNEWLY EMITTING  ({len(newly)} documents)")
        print(f"  definitions {defs}, of which {refs} referenced ({100 * refs / max(1, defs):.0f}%)")

    lost = [k for k in shared if emits(before[k]) and not emits(after[k])]
    if lost:
        print(f"\nREGRESSED TO NO FOOTNOTES: {len(lost)} documents")
        for key in lost:
            print(f"    {key}")

    ok = (a_total - b_total) <= args.max_regression and not lost
    print(f"\nverdict: {'quality preserved' if ok else 'REGRESSION'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
