#!/usr/bin/env python3
"""Document-level verifier for LawPDF Review Mode Markdown.

This checks reconstructed Markdown against structural invariants that law
review articles satisfy by construction, so no human labels are required:

  * footnote numbers run 1..N monotonically across the article
  * every emitted reference has exactly one definition and vice versa
  * a footnote marker never opens a body paragraph
  * heading outlines nest legally and do not contain citation prose
  * running headers, table-of-contents leaders, and page numbers do not
    survive into the body

Every defect is reported with a file:line anchor. A document "passes" only when
it has zero critical defects; `--reference` additionally measures word recall
against an external extraction (e.g. LightOnOCR-2 Markdown) to catch dropped
content, which no internal invariant can see.

Usage:
    python tools/md_verify.py agentic-review-corpus/markdown/*.md
    python tools/md_verify.py doc.md --reference ocr/doc.md --json report.json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Sequence

# ---------------------------------------------------------------------------
# Severity model. Critical defects fail the document; warnings are reported but
# do not gate, because some are legitimate in unusual articles.
# ---------------------------------------------------------------------------

CRITICAL = "critical"
WARNING = "warning"

# A body paragraph longer than this is almost always two paragraphs fused at a
# lost boundary. Legal prose is long, so the threshold is deliberately high.
FUSED_PARAGRAPH_WORDS = 400
# Below this, a standalone body block is usually a stranded fragment.
FRAGMENT_PARAGRAPH_WORDS = 4
# Headings longer than this are usually body text misclassified, or two
# headings fused into one block.
LONG_HEADING_WORDS = 18

PRIVATE_USE = re.compile(r"[-]")
# LawPDF's own callout/marker sentinels, which must never reach the output.
LAWPDF_SENTINEL = re.compile(r"[]")
FOOTNOTE_DEF = re.compile(r"^\[\^([^\]]+)\]:\s*(.*)$")
FOOTNOTE_REF = re.compile(r"\[\^([^\]]+)\]")
HEADING = re.compile(r"^(#{1,6})\s+(.*)$")
TOC_LEADER = re.compile(r"\.{4,}\s*\d+\s*$")
PAGE_NUMBER_ONLY = re.compile(r"^\[?\d{1,4}\]?$")
RUNNING_HEADER = re.compile(
    r"(\[Vol\.\s*\d+|\bL\.\s*REV\.|LAW REVIEW\b|\bJOURNAL OF\b.*\d{4})", re.IGNORECASE
)
# "126 See, e.g., ..." or "105. Richard A. Posner, ..." — footnote text left in
# the body as plain prose instead of becoming a [^n]: definition. The number is
# often a citation volume rather than the note number, so the detail line does
# not claim to know which.
BARE_NOTE_OPENER = re.compile(r"^(\d{1,3})\.?\s+(?=[A-Z(“\"])")
ENUMERATOR = re.compile(
    r"(?:^|\s)(?:[IVXLC]+\.|[A-Z]\.|\d+\.|[a-z]\.)(?=\s+\S)"
)
CASE_CITATION = re.compile(r"\b\d+\s+[A-Z][A-Za-z.]*\s+\d+|\bv\.\s+[A-Z]")
WORD = re.compile(r"[0-9a-zÀ-ɏ]+")


@dataclass
class Defect:
    kind: str
    severity: str
    line: int
    detail: str

    def to_json(self) -> dict:
        return {
            "kind": self.kind,
            "severity": self.severity,
            "line": self.line,
            "detail": self.detail,
        }


@dataclass
class Block:
    """A blank-line-delimited chunk of Markdown with its 1-based start line."""

    line: int
    lines: list[str]

    @property
    def text(self) -> str:
        return "\n".join(self.lines)

    @property
    def first(self) -> str:
        return self.lines[0] if self.lines else ""


@dataclass
class DocumentReport:
    path: str
    defects: list[Defect] = field(default_factory=list)
    stats: dict = field(default_factory=dict)

    def add(self, kind: str, severity: str, line: int, detail: str) -> None:
        self.defects.append(Defect(kind, severity, line, detail))

    @property
    def critical(self) -> list[Defect]:
        return [d for d in self.defects if d.severity == CRITICAL]

    @property
    def warnings(self) -> list[Defect]:
        return [d for d in self.defects if d.severity == WARNING]

    @property
    def passed(self) -> bool:
        return not self.critical

    def to_json(self) -> dict:
        return {
            "path": self.path,
            "passed": self.passed,
            "critical_count": len(self.critical),
            "warning_count": len(self.warnings),
            "stats": self.stats,
            "defects": [d.to_json() for d in self.defects],
        }


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------


def split_blocks(lines: Sequence[str]) -> list[Block]:
    blocks: list[Block] = []
    current: list[str] = []
    start = 1
    in_fence = False
    for index, raw in enumerate(lines, start=1):
        line = raw.rstrip("\n")
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
        if not line.strip() and not in_fence:
            if current:
                blocks.append(Block(start, current))
                current = []
            continue
        if not current:
            start = index
        current.append(line)
    if current:
        blocks.append(Block(start, current))
    return blocks


def is_footnote_def(block: Block) -> bool:
    return bool(FOOTNOTE_DEF.match(block.first))


def is_heading(block: Block) -> bool:
    return bool(HEADING.match(block.first))


def is_table_or_code(block: Block) -> bool:
    first = block.first.lstrip()
    return first.startswith("```") or first.startswith("|")


def is_list_item(block: Block) -> bool:
    return bool(re.match(r"^\s*([*+-]|\d+[.)])\s+", block.first))


def word_count(text: str) -> int:
    return len(WORD.findall(text.lower()))


def strip_markup(text: str) -> str:
    text = FOOTNOTE_REF.sub(" ", text)
    text = re.sub(r"[*_`#>]+", " ", text)
    return text


# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------


def check_footnotes(blocks: Sequence[Block], report: DocumentReport) -> None:
    definitions: list[tuple[str, int]] = []
    seen_ids: dict[str, int] = {}
    for block in blocks:
        match = FOOTNOTE_DEF.match(block.first)
        if not match:
            continue
        ident = match.group(1)
        if ident in seen_ids:
            report.add(
                "footnote.duplicate_definition",
                CRITICAL,
                block.line,
                f"note {ident} is defined again (first at line {seen_ids[ident]})",
            )
            continue
        seen_ids[ident] = block.line
        definitions.append((ident, block.line))
        if not match.group(2).strip() and len(block.lines) == 1:
            report.add(
                "footnote.empty_definition",
                CRITICAL,
                block.line,
                f"note {ident} has no text",
            )

    references: dict[str, list[int]] = {}
    for block in blocks:
        if is_footnote_def(block):
            continue
        for offset, line in enumerate(block.lines):
            for ident in FOOTNOTE_REF.findall(line):
                references.setdefault(ident, []).append(block.line + offset)

    defined = set(seen_ids)
    referenced = set(references)

    for ident in sorted(referenced - defined):
        report.add(
            "footnote.unresolved_reference",
            CRITICAL,
            references[ident][0],
            f"reference [^{ident}] has no definition",
        )
    for ident in sorted(defined - referenced):
        report.add(
            "footnote.orphan_definition",
            CRITICAL,
            seen_ids[ident],
            f"note {ident} is defined but never referenced in the body",
        )

    numeric = [(int(i), line) for i, line in definitions if i.isdigit()]
    report.stats["footnote_definitions"] = len(definitions)
    report.stats["footnote_references"] = len(referenced)
    report.stats["footnote_numeric_definitions"] = len(numeric)

    if not numeric:
        return

    numbers = [n for n, _ in numeric]
    line_of = {n: line for n, line in numeric}
    lowest, highest = min(numbers), max(numbers)
    report.stats["footnote_range"] = [lowest, highest]

    if lowest != 1:
        report.add(
            "footnote.sequence_start",
            WARNING,
            line_of[lowest],
            f"numbering starts at {lowest}, not 1",
        )

    missing = sorted(set(range(lowest, highest + 1)) - set(numbers))
    report.stats["footnote_missing"] = missing
    if missing:
        report.add(
            "footnote.sequence_gap",
            CRITICAL,
            line_of[highest],
            f"{len(missing)} note(s) missing from {lowest}..{highest}: "
            + summarize_runs(missing),
        )

    for previous, current in zip(numeric, numeric[1:]):
        if current[0] < previous[0]:
            report.add(
                "footnote.sequence_disorder",
                CRITICAL,
                current[1],
                f"note {current[0]} is defined after note {previous[0]}",
            )
            break


def check_body(blocks: Sequence[Block], report: DocumentReport) -> None:
    body_words: list[int] = []

    # A running header repeats on every page; a legal citation that merely
    # mentions a reporter does not. Counting occurrences separates them.
    from collections import Counter

    repeats = Counter(
        re.sub(r"\s+", " ", block.first).strip().lower()
        for block in blocks
        if len(block.lines) == 1 and not is_footnote_def(block)
    )
    for block in blocks:
        if is_heading(block) or is_footnote_def(block) or is_table_or_code(block):
            continue

        text = block.text
        stripped = text.strip()

        if LAWPDF_SENTINEL.search(text):
            report.add(
                "furniture.private_sentinel",
                CRITICAL,
                block.line,
                "LawPDF callout/marker sentinel leaked into output",
            )
        elif (glyph := PRIVATE_USE.search(text)) is not None:
            # Symbol/Wingdings glyphs the PDF encodes in the private-use area,
            # e.g. U+F0B7 for a bullet. They render as tofu and should be
            # mapped to real Unicode.
            report.add(
                "furniture.unmapped_glyph",
                WARNING,
                block.line,
                f"unmapped private-use glyph U+{ord(glyph.group()):04X}: "
                f"{preview(stripped)}",
            )

        if TOC_LEADER.search(stripped):
            report.add(
                "furniture.toc_leader",
                CRITICAL,
                block.line,
                f"table-of-contents line survived into body: {preview(stripped)}",
            )
            continue

        if PAGE_NUMBER_ONLY.match(stripped):
            report.add(
                "furniture.page_number",
                CRITICAL,
                block.line,
                f"standalone page number: {preview(stripped)}",
            )
            continue

        if is_list_item(block):
            continue

        # A marker that opens a paragraph belongs to the previous paragraph's
        # last sentence; this is the "unattached note number" defect.
        if re.match(r"^\[\^[^\]]+\]", stripped):
            report.add(
                "footnote.marker_opens_paragraph",
                CRITICAL,
                block.line,
                f"paragraph begins with a footnote marker: {preview(stripped)}",
            )

        # Only meaningful once the document is known to use numbered notes;
        # otherwise a numbered list would trip it.
        if report.stats.get("footnote_numeric_definitions") and BARE_NOTE_OPENER.match(
            stripped
        ):
            report.add(
                "footnote.definition_in_body",
                CRITICAL,
                block.line,
                f"footnote text emitted as a body paragraph: {preview(stripped)}",
            )

        occurrences = repeats[re.sub(r"\s+", " ", stripped).lower()]
        if (
            len(block.lines) == 1
            and RUNNING_HEADER.search(stripped)
            and word_count(stripped) < 15
            and occurrences >= 3
        ):
            report.add(
                "furniture.running_header",
                CRITICAL,
                block.line,
                f"running header survived into body ({occurrences} times): "
                f"{preview(stripped)}",
            )
            continue

        count = word_count(strip_markup(text))
        body_words.append(count)
        if count >= FUSED_PARAGRAPH_WORDS:
            report.add(
                "paragraph.suspected_fusion",
                CRITICAL,
                block.line,
                f"{count}-word paragraph, likely two paragraphs fused",
            )
        elif count and count <= FRAGMENT_PARAGRAPH_WORDS:
            report.add(
                "paragraph.fragment",
                WARNING,
                block.line,
                f"{count}-word standalone paragraph: {preview(stripped)}",
            )

    report.stats["body_paragraphs"] = len(body_words)
    report.stats["body_words"] = sum(body_words)
    if body_words:
        ordered = sorted(body_words)
        report.stats["body_words_median"] = ordered[len(ordered) // 2]
        report.stats["body_words_max"] = ordered[-1]


def check_headings(blocks: Sequence[Block], report: DocumentReport) -> None:
    headings: list[tuple[int, str, int]] = []
    for block in blocks:
        match = HEADING.match(block.first)
        if not match:
            continue
        level = len(match.group(1))
        title = match.group(2).strip()
        headings.append((level, title, block.line))

    report.stats["headings"] = len(headings)
    if not headings:
        report.add("heading.absent", CRITICAL, 1, "document has no headings")
        return

    seen: dict[str, int] = {}
    previous_level = headings[0][0]
    for level, title, line in headings:
        if level > previous_level + 1:
            report.add(
                "heading.level_jump",
                WARNING,
                line,
                f"H{previous_level} is followed by H{level}: {preview(title)}",
            )
        previous_level = level

        normalized = re.sub(r"\s+", " ", title.lower()).strip(" .,;:")
        if normalized in seen:
            report.add(
                "heading.duplicate",
                WARNING,
                line,
                f"repeats the heading at line {seen[normalized]}: {preview(title)}",
            )
        else:
            seen[normalized] = line

        if not title:
            report.add("heading.empty", CRITICAL, line, "empty heading")
            continue

        if title.endswith((",", ";", ":")) or title.endswith(" and"):
            report.add(
                "heading.unterminated",
                CRITICAL,
                line,
                f"heading ends mid-clause, likely body text: {preview(title)}",
            )

        if CASE_CITATION.search(title) and word_count(title) <= 8:
            report.add(
                "heading.citation_prose",
                CRITICAL,
                line,
                f"heading looks like a citation: {preview(title)}",
            )

        if TOC_LEADER.search(title):
            report.add(
                "heading.toc_leader",
                CRITICAL,
                line,
                f"heading is a table-of-contents entry: {preview(title)}",
            )

        if FOOTNOTE_REF.search(title):
            report.add(
                "heading.contains_marker",
                WARNING,
                line,
                f"heading carries a footnote marker: {preview(title)}",
            )

        words = word_count(title)
        if words > LONG_HEADING_WORDS:
            report.add(
                "heading.overlong",
                CRITICAL,
                line,
                f"{words}-word heading, likely body text or fused headings: "
                f"{preview(title)}",
            )
        elif len(ENUMERATOR.findall(" " + title)) > 1:
            report.add(
                "heading.fused",
                CRITICAL,
                line,
                f"heading contains two section enumerators: {preview(title)}",
            )


# ---------------------------------------------------------------------------
# Content-recall checks.
#
# The invariant checks above only see text that survived into the Markdown, so
# they are blind to the largest defect class: source text that was dropped
# outright. These compare against an external rendition of the same document.
#
# Comparison is multiset-based, not sequence-based, because the generator moves
# every footnote definition to the end of the document; sequence alignment
# would report that reordering as massive loss.
# ---------------------------------------------------------------------------

# Below this share of source tokens surviving, the export is losing content.
SOURCE_RECALL_FLOOR = 0.95
# A dropped source line shorter than this is usually furniture, not content.
MIN_PROBE_TOKENS = 4
# Distance below the page's median font size that marks the footnote zone.
FOOTNOTE_FONT_DELTA = 0.4
# Dropped lines are reported as a total plus this many worked examples.
DROPPED_LINE_SAMPLES = 5


def normalize_words(text: str) -> list[str]:
    text = unicodedata.normalize("NFKC", text).replace("­", "")
    text = text.replace("’", "'").replace("“", '"').replace("”", '"')
    # Rejoin words split across a line break by a hyphen.
    text = re.sub(r"(\w)[-‐‑]\s*\n\s*(\w)", r"\1\2", text)
    return WORD.findall(text.lower())


def multiset_recall(source: Sequence[str], output: Sequence[str]) -> tuple[int, int]:
    from collections import Counter

    have = Counter(source)
    kept = sum((have & Counter(output)).values())
    return kept, sum(have.values())


def report_recall(
    report: DocumentReport,
    label: str,
    kept: int,
    total: int,
    severity: str = CRITICAL,
) -> float | None:
    if not total:
        return None
    recall = kept / total
    report.stats[f"{label}_recall"] = round(recall, 4)
    report.stats[f"{label}_tokens_lost"] = total - kept
    if recall < SOURCE_RECALL_FLOOR:
        report.add(
            f"recall.{label}_loss",
            severity,
            1,
            f"{1 - recall:.1%} of {label} text ({total - kept} of {total} tokens) "
            "is absent from the export",
        )
    return recall


def check_reference_recall(text: str, reference: str, report: DocumentReport) -> None:
    """Compare against another Markdown rendition (e.g. LightOnOCR-2)."""
    ours = normalize_words(text)
    theirs = normalize_words(reference)
    if not theirs:
        report.add("recall.empty_reference", WARNING, 1, "reference has no words")
        return
    report.stats["reference_tokens"] = len(theirs)
    kept, total = multiset_recall(theirs, ours)
    report_recall(report, "reference", kept, total)


def check_source_recall(pdf_path: Path, text: str, report: DocumentReport) -> None:
    """Compare against the original PDF's own text layer, split by zone.

    Body and footnote zones are separated by font size so the report says which
    zone is losing content: they have different failure modes and different
    fixes.
    """
    try:
        import fitz  # PyMuPDF
    except ImportError:
        report.add(
            "recall.unavailable",
            WARNING,
            1,
            "PyMuPDF is not installed; source recall was skipped",
        )
        return

    try:
        document = fitz.open(pdf_path)
    except Exception as error:  # noqa: BLE001 - report, do not abort the run
        report.add("recall.unreadable_source", WARNING, 1, f"{pdf_path.name}: {error}")
        return

    output = normalize_words(text)
    output_probe = " ".join(output)

    # Partition the export the same way the source is partitioned, so each zone
    # is compared against its counterpart rather than against the whole
    # document (which would let common words match in both and overstate
    # recall).
    output_zones: dict[str, list[str]] = {"body": [], "footnote": []}
    for block in split_blocks(text.splitlines()):
        zone = "footnote" if is_footnote_def(block) else "body"
        output_zones[zone].extend(normalize_words(block.text))

    zoned: dict[str, list[str]] = {"body": [], "footnote": []}
    dropped: list[tuple[int, int, str]] = []
    all_tokens: list[str] = []

    with document:
        report.stats["source_pages"] = document.page_count
        for page_number, page in enumerate(document, start=1):
            lines: list[tuple[float, str]] = []
            for block in page.get_text("dict")["blocks"]:
                for line in block.get("lines", []):
                    spans = line.get("spans", [])
                    content = "".join(span["text"] for span in spans)
                    sizes = [span["size"] for span in spans if span["text"].strip()]
                    if content.strip() and sizes:
                        lines.append((max(sizes), content))
            if not lines:
                continue

            body_size = sorted(size for size, _ in lines)[len(lines) // 2]
            for size, content in lines:
                tokens = normalize_words(content)
                all_tokens.extend(tokens)
                zone = "footnote" if size < body_size - FOOTNOTE_FONT_DELTA else "body"
                zoned[zone].extend(tokens)
                if len(tokens) < MIN_PROBE_TOKENS:
                    continue
                # Probe from the line's interior: the first tokens may have been
                # absorbed by dehyphenation of the previous line.
                probe = " ".join(tokens[1:7])
                if probe not in output_probe:
                    dropped.append((page_number, len(tokens), content.strip()))

    kept, total = multiset_recall(all_tokens, output)
    report_recall(report, "source", kept, total)
    for zone in ("body", "footnote"):
        zone_kept, zone_total = multiset_recall(zoned[zone], output_zones[zone])
        report_recall(report, zone, zone_kept, zone_total, severity=WARNING)

    if not dropped:
        return
    report.stats["dropped_source_lines"] = len(dropped)
    dropped.sort(key=lambda item: item[1], reverse=True)
    # One entry carries the true total; the rest are labelled samples, so the
    # defect table can never read as though only a handful of lines were lost.
    report.add(
        "recall.dropped_lines_total",
        WARNING,
        1,
        f"{len(dropped)} source line(s) absent from the export "
        f"({sum(count for _, count, _ in dropped)} tokens); "
        f"{min(len(dropped), DROPPED_LINE_SAMPLES)} shown",
    )
    for page_number, count, content in dropped[:DROPPED_LINE_SAMPLES]:
        report.add(
            "recall.dropped_line_sample",
            WARNING,
            1,
            f"p{page_number} [{count} tokens] {preview(content)}",
        )


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def preview(text: str, limit: int = 90) -> str:
    flat = re.sub(r"\s+", " ", text).strip()
    return flat if len(flat) <= limit else flat[: limit - 1] + "…"


def summarize_runs(numbers: Sequence[int], limit: int = 6) -> str:
    runs: list[str] = []
    start = previous = numbers[0]
    for value in numbers[1:]:
        if value == previous + 1:
            previous = value
            continue
        runs.append(str(start) if start == previous else f"{start}-{previous}")
        start = previous = value
    runs.append(str(start) if start == previous else f"{start}-{previous}")
    if len(runs) > limit:
        return ", ".join(runs[:limit]) + f", +{len(runs) - limit} more"
    return ", ".join(runs)


def verify(path: Path, reference: Path | None, source: Path | None) -> DocumentReport:
    text = path.read_text(encoding="utf-8", errors="replace")
    report = DocumentReport(path=str(path))
    blocks = split_blocks(text.splitlines())
    report.stats["blocks"] = len(blocks)

    check_footnotes(blocks, report)
    check_body(blocks, report)
    check_headings(blocks, report)
    if reference is not None:
        check_reference_recall(
            text, reference.read_text(encoding="utf-8", errors="replace"), report
        )
    if source is not None:
        check_source_recall(source, text, report)

    report.defects.sort(key=lambda d: (d.line, d.kind))
    return report


def print_report(report: DocumentReport, verbose: bool) -> None:
    status = "PASS" if report.passed else "FAIL"
    name = Path(report.path).name
    print(f"{status}  {name}")
    stats = report.stats
    print(
        "      {paras} paragraphs, {heads} headings, {defs} notes{rng}{recall}".format(
            paras=stats.get("body_paragraphs", 0),
            heads=stats.get("headings", 0),
            defs=stats.get("footnote_definitions", 0),
            rng=(
                f" ({stats['footnote_range'][0]}..{stats['footnote_range'][1]})"
                if stats.get("footnote_range")
                else ""
            ),
            recall=(
                ", recall {source:.0%} source / {body:.0%} body / {fn:.0%} notes".format(
                    source=stats["source_recall"],
                    body=stats.get("body_recall", 0.0),
                    fn=stats.get("footnote_recall", 0.0),
                )
                if "source_recall" in stats
                else (
                    f", recall {stats['reference_recall']:.0%} vs reference"
                    if "reference_recall" in stats
                    else ""
                )
            ),
        )
    )

    grouped: dict[str, list[Defect]] = {}
    for defect in report.defects:
        grouped.setdefault(defect.kind, []).append(defect)

    for kind in sorted(grouped, key=lambda k: (-len(grouped[k]), k)):
        items = grouped[kind]
        mark = "!" if items[0].severity == CRITICAL else "-"
        print(f"    {mark} {kind} x{len(items)}")
        shown = items if verbose else items[:3]
        for defect in shown:
            print(f"        {report.path}:{defect.line}  {defect.detail}")
        if len(items) > len(shown):
            print(f"        ... {len(items) - len(shown)} more")


def main(argv: Iterable[str]) -> int:
    # Defect details quote source text, which routinely contains characters the
    # Windows console codepage cannot encode. Redirecting stdout would
    # otherwise abort the run partway through.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")

    parser = argparse.ArgumentParser(
        description="Verify structural integrity of LawPDF Markdown exports."
    )
    parser.add_argument("paths", nargs="+", type=Path, help="Markdown files to check")
    parser.add_argument(
        "--reference",
        type=Path,
        help="Directory of reference extractions (e.g. LightOnOCR-2 Markdown), "
        "matched by file stem, or a single file when one path is checked",
    )
    parser.add_argument(
        "--source",
        type=Path,
        help="Directory of original PDFs (or a single PDF), matched by file "
        "stem, used to measure how much source text the export dropped",
    )
    parser.add_argument("--json", type=Path, help="Write the full report as JSON")
    parser.add_argument(
        "--verbose", action="store_true", help="List every defect, not the first three"
    )
    args = parser.parse_args(list(argv))

    targets: list[Path] = []
    for path in args.paths:
        targets.extend(sorted(path.glob("*.md")) if path.is_dir() else [path])

    def companion(root: Path | None, stem: str, suffix: str) -> Path | None:
        if root is None:
            return None
        if root.is_dir():
            candidate = root / f"{stem}{suffix}"
            return candidate if candidate.exists() else None
        return root if len(targets) == 1 else None

    reports: list[DocumentReport] = []
    for target in targets:
        reports.append(
            verify(
                target,
                companion(args.reference, target.stem, ".md"),
                companion(args.source, target.stem, ".pdf"),
            )
        )

    for report in reports:
        print_report(report, args.verbose)
        print()

    passed = sum(1 for report in reports if report.passed)
    critical = sum(len(report.critical) for report in reports)
    warnings = sum(len(report.warnings) for report in reports)
    print(
        f"{passed}/{len(reports)} documents clean; "
        f"{critical} critical defects, {warnings} warnings"
    )

    recalls = [r.stats["source_recall"] for r in reports if "source_recall" in r.stats]
    if recalls:
        lost = sum(r.stats.get("source_tokens_lost", 0) for r in reports)
        print(
            f"source recall: mean {sum(recalls) / len(recalls):.1%}, "
            f"worst {min(recalls):.1%}, {lost} tokens dropped in total"
        )

    by_kind: dict[str, int] = {}
    for report in reports:
        for defect in report.defects:
            by_kind[defect.kind] = by_kind.get(defect.kind, 0) + 1
    if by_kind:
        print("\ndefect totals:")
        for kind, count in sorted(by_kind.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:5d}  {kind}")

    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "documents_total": len(reports),
                    "documents_clean": passed,
                    "critical_defects": critical,
                    "warnings": warnings,
                    "defects_by_kind": by_kind,
                    "documents": [report.to_json() for report in reports],
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        print(f"\nwrote {args.json}")

    return 0 if critical == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
