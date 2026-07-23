#!/usr/bin/env python3
"""Build the five-article LawPDF agentic-review corpus.

The input diagnostics are emitted by LawPDF's LM2 draft diagnostics. The
sidecars and Markdown files come from completed Review Mode documents. This
script deliberately keeps runtime classifier evidence separate from a bundled
CatBoost model evaluated in shadow mode.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import re
import shutil
import tempfile
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import numpy as np
from catboost import CatBoostClassifier, Pool


ARTICLES = (
    {
        "id": "011",
        "slug": "co-governance",
        "ordinal": 1,
        "qa_tier": "known_hard_case",
        "qa_note": (
            "The generator omitted 19 standalone footnote-separator fragments, "
            "but two separators and note fragments remain fused into body "
            "paragraphs upstream."
        ),
    },
    {
        "id": "037",
        "slug": "cost-of-justice-ai",
        "ordinal": 2,
        "qa_tier": "best_available_with_known_artifacts",
        "qa_note": (
            "Lowercase alphabetic subsections correctly map to H4 and linked "
            "footnotes resolve, but a few unattached numeric note markers remain "
            "at paragraph starts (including 126, 130, and 161)."
        ),
    },
    {
        "id": "042",
        "slug": "beyond-intent",
        "ordinal": 3,
        "qa_tier": "known_hard_case",
        "qa_note": (
            "Includes a suspected upstream heading-role false positive around "
            "\"Massachusetts v. Feeney\"."
        ),
    },
    {
        "id": "047",
        "slug": "antisemitism-title-vi",
        "ordinal": 4,
        "qa_tier": "known_hard_case",
        "qa_note": (
            "The generator omitted 20 standalone footnote-separator fragments, "
            "but unattached numeric note markers remain at paragraph starts "
            "(including 16, 30, and 131)."
        ),
    },
    {
        "id": "057",
        "slug": "contract-wrapped-property",
        "ordinal": 5,
        "qa_tier": "known_hard_case",
        "qa_note": (
            "Includes suspected upstream heading-role false positives in "
            "warranty and citation prose."
        ),
    },
)

CSV_COLUMNS = (
    "article_id",
    "article_title",
    "source_pdf",
    "page_number",
    "page_index",
    "line_index",
    "line_id",
    "text",
    "final_block_index",
    "final_block_role",
    "role_hint",
    "active_runtime_model",
    "runtime_action",
    "runtime_action_confidence",
    "runtime_emission_argmax_action",
    "runtime_emission_argmax_confidence",
    "runtime_score_hide_noise",
    "runtime_score_keep",
    "runtime_score_marginalia",
    "shadow_model",
    "shadow_catboost_action",
    "shadow_catboost_confidence",
    "shadow_prob_hide_noise",
    "shadow_prob_keep",
    "shadow_prob_marginalia",
    "runtime_shadow_disagreement",
    "review_priority",
    "top_feature_1_name",
    "top_feature_1_value",
    "top_feature_1_shap",
    "top_feature_2_name",
    "top_feature_2_value",
    "top_feature_2_shap",
    "top_feature_3_name",
    "top_feature_3_value",
    "top_feature_3_shap",
    "pc1",
    "pc2",
    "pc3",
    "y_bottom_ratio",
    "font_ratio_page",
    "font_ratio_doc",
    "below_footnote_divider",
    "page_has_footnote_divider",
    "doc_note_marker",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--diagnostics", type=Path, required=True)
    parser.add_argument("--sidecar-dir", type=Path, required=True)
    parser.add_argument("--markdown-dir", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_text_lf(path: Path, text: str, *, encoding: str = "utf-8") -> None:
    with path.open("w", encoding=encoding, newline="\n") as stream:
        stream.write(text)


def find_one(directory: Path, pattern: str) -> Path:
    matches = sorted(directory.glob(pattern))
    if len(matches) != 1:
        raise RuntimeError(
            f"expected exactly one match for {pattern!r} in {directory}, "
            f"found {len(matches)}"
        )
    return matches[0]


def scalar(value: Any) -> Any:
    if isinstance(value, (np.floating, np.integer)):
        return value.item()
    return value


def rounded(value: Any, digits: int = 9) -> float:
    return round(float(value), digits)


def private_marker_characters(text: str) -> list[str]:
    return sorted({f"U+{ord(ch):04X}" for ch in text if 0xE000 <= ord(ch) <= 0xF8FF})


def markdown_metrics(text: str) -> dict[str, Any]:
    definition_ids = set(re.findall(r"(?m)^\[\^([^\]]+)\]:", text))
    all_reference_ids = set(re.findall(r"\[\^([^\]]+)\]", text))
    heading_levels = Counter(
        len(match.group(1))
        for match in re.finditer(r"(?m)^(#{1,6})\s+\S", text)
    )
    return {
        "bytes_utf8": len(text.encode("utf-8")),
        "heading_count": sum(heading_levels.values()),
        "heading_counts_by_level": {
            str(level): heading_levels.get(level, 0) for level in range(1, 7)
        },
        "footnote_definition_count": len(definition_ids),
        "footnote_reference_id_count": len(all_reference_ids),
        "unresolved_footnote_ids": sorted(all_reference_ids - definition_ids),
        "private_use_marker_characters": private_marker_characters(text),
    }


def stable_pca(
    rows: list[dict[str, Any]], float_names: list[str]
) -> tuple[np.ndarray, dict[str, Any]]:
    matrix = np.asarray(
        [[float(row["float_features"][name]) for name in float_names] for row in rows],
        dtype=np.float64,
    )
    if not np.isfinite(matrix).all():
        raise RuntimeError("PCA input contains non-finite values")

    means = matrix.mean(axis=0)
    standard_deviations = matrix.std(axis=0, ddof=0)
    variable_mask = standard_deviations > 1e-12
    retained_names = [
        name for name, keep in zip(float_names, variable_mask, strict=True) if keep
    ]
    standardized = (
        matrix[:, variable_mask] - means[variable_mask]
    ) / standard_deviations[variable_mask]

    _, singular_values, vt = np.linalg.svd(standardized, full_matrices=False)
    components = vt[:3].copy()
    for index in range(components.shape[0]):
        pivot = int(np.argmax(np.abs(components[index])))
        if components[index, pivot] < 0:
            components[index] *= -1

    coordinates = standardized @ components.T
    variance = singular_values**2 / max(1, len(rows) - 1)
    explained_ratio = variance / variance.sum()
    component_metadata = []
    for component_index, loadings in enumerate(components):
        top_indices = np.argsort(np.abs(loadings))[::-1][:10]
        component_metadata.append(
            {
                "component": f"PC{component_index + 1}",
                "explained_variance_ratio": rounded(
                    explained_ratio[component_index], 12
                ),
                "top_loadings": [
                    {
                        "feature": retained_names[int(feature_index)],
                        "loading": rounded(loadings[int(feature_index)], 12),
                    }
                    for feature_index in top_indices
                ],
            }
        )

    return coordinates, {
        "method": "PCA via NumPy SVD",
        "scope": "all source lines across all five articles",
        "input": "116 exact numeric LM2/CatBoost features",
        "standardization": "population mean 0 and standard deviation 1",
        "constant_features_removed": [
            name
            for name, keep in zip(float_names, variable_mask, strict=True)
            if not keep
        ],
        "retained_feature_count": len(retained_names),
        "components": component_metadata,
        "interpretation": (
            "PCA describes global feature variance, not classifier decision "
            "importance. Use the per-line SHAP columns for prediction attribution."
        ),
    }


def catboost_matrix(
    rows: list[dict[str, Any]], feature_names: list[str]
) -> list[list[Any]]:
    matrix = []
    for row in rows:
        values = []
        for name in feature_names:
            if name == "catboost_text":
                values.append(row["catboost_text"])
            elif name in row["float_features"]:
                values.append(float(row["float_features"][name]))
            elif name in row["categorical_features"]:
                values.append(str(row["categorical_features"][name]))
            else:
                raise RuntimeError(f"missing CatBoost feature {name!r}")
        matrix.append(values)
    return matrix


def line_to_block_map(sidecar: dict[str, Any]) -> dict[str, dict[str, Any]]:
    mapping: dict[str, dict[str, Any]] = {}
    for block in sidecar["blocks"]:
        for line_id in block.get("source_line_ids", []):
            mapping.setdefault(
                line_id,
                {
                    "final_block_index": block["block_index"],
                    "final_block_role": block["role"],
                },
            )
    return mapping


def review_priority(
    runtime_confidence: float, disagreement: bool, shadow_confidence: float
) -> str:
    if runtime_confidence < 0.60 or (disagreement and shadow_confidence >= 0.80):
        return "high"
    if runtime_confidence < 0.80 or disagreement:
        return "medium"
    return "normal"


def build_csv(
    path: Path,
    *,
    article: dict[str, Any],
    title: str,
    rows: list[dict[str, Any]],
    global_indices: list[int],
    pca_coordinates: np.ndarray,
    block_mapping: dict[str, dict[str, Any]],
    runtime_model_label: str,
    model_label: str,
    model: CatBoostClassifier,
    feature_names: list[str],
    class_names: list[str],
) -> dict[str, Any]:
    cat_feature_indices = [
        index
        for index, name in enumerate(feature_names)
        if name in rows[0]["categorical_features"]
    ]
    text_feature_indices = [
        index for index, name in enumerate(feature_names) if name == "catboost_text"
    ]
    pool = Pool(
        catboost_matrix(rows, feature_names),
        cat_features=cat_feature_indices,
        text_features=text_feature_indices,
        feature_names=feature_names,
    )
    probabilities = np.asarray(model.predict_proba(pool), dtype=np.float64)
    shap_values = np.asarray(
        model.get_feature_importance(pool, type="ShapValues"), dtype=np.float64
    )
    if shap_values.shape != (len(rows), len(class_names), len(feature_names) + 1):
        raise RuntimeError(f"unexpected SHAP shape: {shap_values.shape}")

    predicted_indices = probabilities.argmax(axis=1)
    runtime_action_counts: Counter[str] = Counter()
    shadow_action_counts: Counter[str] = Counter()
    priority_counts: Counter[str] = Counter()
    disagreement_count = 0

    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(
            stream,
            fieldnames=CSV_COLUMNS,
            extrasaction="raise",
            lineterminator="\n",
        )
        writer.writeheader()

        for local_index, (row, global_index) in enumerate(
            zip(rows, global_indices, strict=True)
        ):
            predicted_index = int(predicted_indices[local_index])
            shadow_action = class_names[predicted_index]
            shadow_confidence = float(probabilities[local_index, predicted_index])
            runtime_action = row["lm2_action"]
            runtime_confidence = float(row["decoded_action_emission_probability"])
            disagreement = runtime_action != shadow_action
            priority = review_priority(
                runtime_confidence, disagreement, shadow_confidence
            )

            selected_shap = shap_values[local_index, predicted_index, :-1]
            top_feature_indices = np.argsort(np.abs(selected_shap))[::-1][:3]
            # Pool.get_features() does not support categorical/text features,
            # so retain the original typed values for attribution display.
            original_feature_values = catboost_matrix([row], feature_names)[0]
            attributions = []
            for feature_index in top_feature_indices:
                integer_index = int(feature_index)
                value = original_feature_values[integer_index]
                attributions.append(
                    {
                        "name": feature_names[integer_index],
                        "value": scalar(value),
                        "shap": rounded(selected_shap[integer_index], 12),
                    }
                )

            block = block_mapping.get(
                row["line_id"],
                {"final_block_index": "", "final_block_role": "hidden"},
            )
            scores = row["emission_scores_after_priors"]
            probability_by_class = {
                name: float(probabilities[local_index, class_index])
                for class_index, name in enumerate(class_names)
            }
            coordinates = pca_coordinates[global_index]
            record = {
                "article_id": article["id"],
                "article_title": title,
                "source_pdf": f"../originals/{article['id']}-{article['slug']}.pdf",
                "page_number": int(row["page_index"]) + 1,
                "page_index": row["page_index"],
                "line_index": row["line_index"],
                "line_id": row["line_id"],
                "text": row["text"],
                "final_block_index": block["final_block_index"],
                "final_block_role": block["final_block_role"],
                "role_hint": row.get("role_hint") or "",
                "active_runtime_model": runtime_model_label,
                "runtime_action": runtime_action,
                "runtime_action_confidence": rounded(runtime_confidence, 12),
                "runtime_emission_argmax_action": row["emission_argmax_action"],
                "runtime_emission_argmax_confidence": rounded(
                    row["emission_argmax_confidence"], 12
                ),
                "runtime_score_hide_noise": scores["hide_noise"],
                "runtime_score_keep": scores["keep"],
                "runtime_score_marginalia": scores["marginalia"],
                "shadow_model": model_label,
                "shadow_catboost_action": shadow_action,
                "shadow_catboost_confidence": rounded(shadow_confidence, 12),
                "shadow_prob_hide_noise": rounded(
                    probability_by_class["hide_noise"], 12
                ),
                "shadow_prob_keep": rounded(probability_by_class["keep"], 12),
                "shadow_prob_marginalia": rounded(
                    probability_by_class["marginalia"], 12
                ),
                "runtime_shadow_disagreement": str(disagreement).lower(),
                "review_priority": priority,
                "top_feature_1_name": attributions[0]["name"],
                "top_feature_1_value": attributions[0]["value"],
                "top_feature_1_shap": attributions[0]["shap"],
                "top_feature_2_name": attributions[1]["name"],
                "top_feature_2_value": attributions[1]["value"],
                "top_feature_2_shap": attributions[1]["shap"],
                "top_feature_3_name": attributions[2]["name"],
                "top_feature_3_value": attributions[2]["value"],
                "top_feature_3_shap": attributions[2]["shap"],
                "pc1": rounded(coordinates[0], 12),
                "pc2": rounded(coordinates[1], 12),
                "pc3": rounded(coordinates[2], 12),
                "y_bottom_ratio": row["y_bottom_ratio"],
                "font_ratio_page": row["font_ratio_page"],
                "font_ratio_doc": row["font_ratio_doc"],
                "below_footnote_divider": str(
                    bool(row["below_footnote_divider"])
                ).lower(),
                "page_has_footnote_divider": str(
                    bool(row["page_has_footnote_divider"])
                ).lower(),
                "doc_note_marker": row["doc_note_marker"],
            }
            writer.writerow(record)

            runtime_action_counts[runtime_action] += 1
            shadow_action_counts[shadow_action] += 1
            priority_counts[priority] += 1
            disagreement_count += int(disagreement)

    return {
        "row_count": len(rows),
        "runtime_action_counts": dict(sorted(runtime_action_counts.items())),
        "shadow_action_counts": dict(sorted(shadow_action_counts.items())),
        "review_priority_counts": dict(sorted(priority_counts.items())),
        "runtime_shadow_disagreement_count": disagreement_count,
        "runtime_shadow_disagreement_rate": rounded(
            disagreement_count / len(rows), 12
        ),
    }


def validate_csv(path: Path, expected_rows: int) -> None:
    with path.open("r", encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream)
        if tuple(reader.fieldnames or ()) != CSV_COLUMNS:
            raise RuntimeError(f"unexpected columns in {path}")
        records = list(reader)

    if len(records) != expected_rows:
        raise RuntimeError(
            f"{path} has {len(records)} rows; expected {expected_rows}"
        )
    identities = {(row["page_index"], row["line_index"], row["line_id"]) for row in records}
    if len(identities) != len(records):
        raise RuntimeError(f"duplicate source-line identity in {path}")

    for row in records:
        runtime_confidence = float(row["runtime_action_confidence"])
        shadow_probabilities = [
            float(row["shadow_prob_hide_noise"]),
            float(row["shadow_prob_keep"]),
            float(row["shadow_prob_marginalia"]),
        ]
        if not 0.0 <= runtime_confidence <= 1.0:
            raise RuntimeError(f"invalid runtime confidence in {path}")
        if any(not 0.0 <= probability <= 1.0 for probability in shadow_probabilities):
            raise RuntimeError(f"invalid shadow probability in {path}")
        if not math.isclose(sum(shadow_probabilities), 1.0, abs_tol=2e-9):
            raise RuntimeError(f"shadow probabilities do not sum to 1 in {path}")
        if not all(
            row[f"top_feature_{rank}_name"] and math.isfinite(
                float(row[f"top_feature_{rank}_shap"])
            )
            for rank in (1, 2, 3)
        ):
            raise RuntimeError(f"invalid SHAP attribution in {path}")
        if not all(math.isfinite(float(row[name])) for name in ("pc1", "pc2", "pc3")):
            raise RuntimeError(f"invalid PCA coordinate in {path}")


def readme_text(manifest: dict[str, Any], analysis: dict[str, Any]) -> str:
    article_rows = []
    for article in manifest["articles"]:
        article_rows.append(
            "| {id} | {title} | {qa_tier} | {source_line_count} | {disagreement:.1%} |".format(
                id=article["id"],
                title=article["title"].replace("|", "\\|"),
                qa_tier=article["qa_tier"],
                source_line_count=article["classification"]["row_count"],
                disagreement=article["classification"][
                    "runtime_shadow_disagreement_rate"
                ],
            )
        )

    pca_rows = []
    for component in analysis["pca"]["components"]:
        features = ", ".join(
            f"{item['feature']} ({item['loading']:+.3f})"
            for item in component["top_loadings"][:3]
        )
        pca_rows.append(
            f"| {component['component']} | "
            f"{component['explained_variance_ratio']:.2%} | {features} |"
        )

    return f"""# LawPDF agentic-review corpus

This corpus pairs five original legal PDFs with LawPDF's final post-Review Mode
Markdown and a machine-readable, one-row-per-source-line classification audit.
It is meant for agentic quality review: an agent can compare source lines,
classifier behavior, final block roles, and Markdown structure without rerunning
PDF extraction.

## Contents

- `originals/`: exactly five source PDFs.
- `markdown/`: exactly five final Markdown exports from completed Review Mode
  documents using LawPDF's default **Copy MD** options.
- `classifications/`: exactly five UTF-8 CSVs, one per article.
- `manifest.json`: article paths, counts, hashes, Markdown metrics, and known QA
  observations.
- `analysis-metadata.json`: model provenance, column definitions, SHAP method,
  and global PCA details.
- `checksums.sha256`: SHA-256 verification for every other file in this folder.

## Critical model distinction

The active Windows runtime used for these extracts identified itself as
`{analysis['active_runtime_model']}`. The `runtime_*` columns are therefore the
actual local decisions, scores, and softmax confidence values used during this
run.

The bundled native CatBoost model could be loaded by Python but was not the
active Windows runtime library. Its output is explicitly labeled
`shadow_catboost_*`. The three `top_feature_*` groups are per-line CatBoost SHAP
contributions for the predicted shadow class. Positive SHAP values support that
class; negative values oppose it. Rank is by absolute contribution.

PCA is not feature importance. The `pc1`–`pc3` columns locate each line in a
global standardized numeric-feature space and are useful for clustering or
sampling unusual lines. Use SHAP, not PCA, to explain an individual prediction.

## Articles

| ID | Article | QA tier | Lines | Runtime/shadow disagreement |
|---|---|---:|---:|---:|
{chr(10).join(article_rows)}

Known artifacts are intentionally included so this evaluation set does not
conceal current failure modes. See each article's `qa_note` in `manifest.json`;
no item should be treated as a pristine gold reference.

## CSV reading guide

Each record is one extracted PDF line. `page_number` is human-friendly and
1-based; `page_index` and `line_index` are 0-based. `final_block_role` is the
role of the final Review Mode block that consumed the line, or `hidden` when the
line was omitted. `runtime_action_confidence` is the active runtime emission
probability for the decoded action; the sequential decoder can choose an action
different from `runtime_emission_argmax_action`.

`review_priority` is a convenience triage field:

- `high`: runtime confidence below 0.60, or a runtime/shadow disagreement where
  the shadow confidence is at least 0.80.
- `medium`: runtime confidence below 0.80, or any remaining disagreement.
- `normal`: neither condition applies.

The CSVs preserve article text with standard RFC-style quoting, including
commas, quotes, and any embedded line breaks.

## PCA summary

| Component | Explained variance | Three largest absolute loadings |
|---|---:|---|
{chr(10).join(pca_rows)}

PCA was fit once across all {manifest['total_source_lines']:,} source lines,
after standardizing the exact numeric model inputs. Full loadings and excluded
constant features are in `analysis-metadata.json`.
"""


def main() -> None:
    args = parse_args()
    for path in (
        args.diagnostics,
        args.sidecar_dir,
        args.markdown_dir,
        args.model,
    ):
        if not path.exists():
            raise FileNotFoundError(path)
    if args.output.exists():
        raise FileExistsError(
            f"refusing to overwrite existing output directory: {args.output}"
        )

    diagnostics = load_json(args.diagnostics)
    rows: list[dict[str, Any]] = diagnostics["rows"]
    if diagnostics["row_count"] != len(rows):
        raise RuntimeError("diagnostics row_count does not match rows")
    if len(ARTICLES) != 5:
        raise RuntimeError("corpus configuration must contain exactly five articles")

    model = CatBoostClassifier()
    model.load_model(str(args.model))
    feature_names = list(model.feature_names_)
    class_names = [str(name) for name in model.classes_]
    if set(class_names) != {"hide_noise", "keep", "marginalia"}:
        raise RuntimeError(f"unexpected model classes: {class_names}")

    first_row = rows[0]
    float_names = list(first_row["float_features"])
    available_names = (
        set(first_row["float_features"])
        | set(first_row["categorical_features"])
        | {"catboost_text"}
    )
    if set(feature_names) != available_names:
        missing = sorted(set(feature_names) - available_names)
        extra = sorted(available_names - set(feature_names))
        raise RuntimeError(f"feature mismatch; missing={missing}, extra={extra}")

    pca_coordinates, pca_metadata = stable_pca(rows, float_names)
    row_indices_by_article: dict[str, list[int]] = {
        article["id"]: [] for article in ARTICLES
    }
    for index, row in enumerate(rows):
        source_name = Path(row["path"]).name
        matches = [
            article["id"]
            for article in ARTICLES
            if source_name.startswith(f"{article['id']}_")
        ]
        if len(matches) != 1:
            raise RuntimeError(f"cannot assign diagnostics row path: {row['path']}")
        row_indices_by_article[matches[0]].append(index)

    model_label = f"{args.model.name} (shadow only)"
    generated_at = datetime.now(timezone.utc).isoformat()
    analysis = {
        "schema_version": 1,
        "generated_at_utc": generated_at,
        "active_runtime_model": diagnostics["model_label"],
        "active_runtime_note": (
            "Actual local runtime used for the extracted actions and confidence "
            "columns. On this Windows run, LawPDF reported its heuristic fallback."
        ),
        "shadow_model": {
            "label": model_label,
            "path_at_generation": str(args.model.resolve()),
            "sha256": sha256(args.model),
            "classes_in_probability_order": class_names,
            "feature_count": len(feature_names),
            "feature_names_in_model_order": feature_names,
            "use": (
                "Offline shadow predictions and per-prediction SHAP attribution; "
                "not represented as the active runtime."
            ),
        },
        "shap": {
            "method": "CatBoost exact ShapValues",
            "scope": "predicted shadow class for each source line",
            "ranking": "three greatest absolute SHAP contributions",
            "sign": (
                "positive supports the predicted class; negative opposes the "
                "predicted class"
            ),
            "base_value_excluded_from_top_features": True,
        },
        "pca": pca_metadata,
        "csv_schema": {
            "version": 1,
            "encoding": "UTF-8",
            "row_unit": "one extracted PDF source line",
            "columns": list(CSV_COLUMNS),
            "boolean_encoding": "lowercase true/false",
            "missing_final_block": (
                "final_block_index is empty and final_block_role is hidden"
            ),
        },
    }

    with tempfile.TemporaryDirectory(
        prefix="lawpdf-agentic-review-", dir=r"C:\tmp"
    ) as temporary_directory:
        root = Path(temporary_directory)
        originals_dir = root / "originals"
        markdown_dir = root / "markdown"
        classifications_dir = root / "classifications"
        originals_dir.mkdir()
        markdown_dir.mkdir()
        classifications_dir.mkdir()

        manifest_articles = []
        for article in ARTICLES:
            sidecar_path = find_one(
                args.sidecar_dir, f"*-{article['id']}-*.sidecar.json"
            )
            markdown_source = find_one(
                args.markdown_dir, f"{article['ordinal']:04d}-*.md"
            )
            markdown_sidecar = load_json(
                find_one(
                    args.markdown_dir,
                    f"{article['ordinal']:04d}-*.sidecar.json",
                )
            )
            sidecar = load_json(sidecar_path)
            source_pdf = Path(sidecar["input_path"])
            if not source_pdf.exists():
                raise FileNotFoundError(source_pdf)

            stem = f"{article['id']}-{article['slug']}"
            pdf_target = originals_dir / f"{stem}.pdf"
            markdown_target = markdown_dir / f"{stem}.md"
            csv_target = classifications_dir / f"{stem}.csv"
            shutil.copy2(source_pdf, pdf_target)
            shutil.copy2(markdown_source, markdown_target)

            article_indices = row_indices_by_article[article["id"]]
            article_rows = [rows[index] for index in article_indices]
            source_line_ids = [line["id"] for line in sidecar["source_lines"]]
            diagnostic_line_ids = [row["line_id"] for row in article_rows]
            if source_line_ids != diagnostic_line_ids:
                raise RuntimeError(
                    f"sidecar/diagnostics line sequence mismatch for {article['id']}"
                )

            classification_metrics = build_csv(
                csv_target,
                article=article,
                title=sidecar["title"],
                rows=article_rows,
                global_indices=article_indices,
                pca_coordinates=pca_coordinates,
                block_mapping=line_to_block_map(sidecar),
                runtime_model_label=diagnostics["model_label"],
                model_label=model_label,
                model=model,
                feature_names=feature_names,
                class_names=class_names,
            )
            validate_csv(csv_target, len(article_rows))

            markdown_text = markdown_target.read_text(encoding="utf-8")
            metrics = markdown_metrics(markdown_text)
            if metrics["unresolved_footnote_ids"]:
                raise RuntimeError(
                    f"unresolved footnote IDs in {markdown_target}: "
                    f"{metrics['unresolved_footnote_ids'][:10]}"
                )
            if metrics["private_use_marker_characters"]:
                raise RuntimeError(
                    f"private marker characters in {markdown_target}: "
                    f"{metrics['private_use_marker_characters']}"
                )

            manifest_articles.append(
                {
                    "id": article["id"],
                    "title": sidecar["title"],
                    "qa_tier": article["qa_tier"],
                    "qa_note": article["qa_note"],
                    "generator_warnings": markdown_sidecar.get("warnings", []),
                    "source_pdf": f"originals/{pdf_target.name}",
                    "final_markdown": f"markdown/{markdown_target.name}",
                    "line_classifications": f"classifications/{csv_target.name}",
                    "source_line_count": len(sidecar["source_lines"]),
                    "final_block_count": len(sidecar["blocks"]),
                    "classification": classification_metrics,
                    "markdown_metrics": metrics,
                    "sha256": {
                        "source_pdf": sha256(pdf_target),
                        "final_markdown": sha256(markdown_target),
                        "line_classifications": sha256(csv_target),
                    },
                }
            )

        manifest = {
            "schema_version": 1,
            "generated_at_utc": generated_at,
            "purpose": (
                "Five-article evidence pack for agentic review of LawPDF "
                "classification and final Markdown quality"
            ),
            "article_count": len(manifest_articles),
            "total_source_lines": sum(
                article["source_line_count"] for article in manifest_articles
            ),
            "artifact_counts": {
                "original_pdf": len(list(originals_dir.glob("*.pdf"))),
                "final_markdown": len(list(markdown_dir.glob("*.md"))),
                "line_classification_csv": len(
                    list(classifications_dir.glob("*.csv"))
                ),
            },
            "articles": manifest_articles,
        }
        if manifest["artifact_counts"] != {
            "original_pdf": 5,
            "final_markdown": 5,
            "line_classification_csv": 5,
        }:
            raise RuntimeError(f"unexpected artifact counts: {manifest['artifact_counts']}")

        write_text_lf(
            root / "manifest.json",
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        )
        write_text_lf(
            root / "analysis-metadata.json",
            json.dumps(analysis, ensure_ascii=False, indent=2) + "\n",
        )
        write_text_lf(root / "README.md", readme_text(manifest, analysis))
        write_text_lf(
            root / ".gitattributes",
            "*.md text eol=lf\n"
            "*.json text eol=lf\n"
            "*.csv text eol=lf\n"
            "*.sha256 text eol=lf\n"
            ".gitattributes text eol=lf\n"
            "originals/*.pdf binary\n",
        )

        checksum_paths = sorted(
            path for path in root.rglob("*") if path.is_file()
        )
        checksum_text = "".join(
            f"{sha256(path)}  {path.relative_to(root).as_posix()}\n"
            for path in checksum_paths
        )
        write_text_lf(root / "checksums.sha256", checksum_text, encoding="ascii")

        # Final validation after all metadata has been written.
        if load_json(root / "manifest.json")["total_source_lines"] != len(rows):
            raise RuntimeError("manifest total does not match diagnostics row count")
        for path in root.rglob("*"):
            if path.is_file() and path.stat().st_size == 0:
                raise RuntimeError(f"empty corpus artifact: {path}")

        args.output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(root, args.output)

    print(
        json.dumps(
            {
                "output": str(args.output.resolve()),
                "articles": 5,
                "source_lines": len(rows),
                "runtime_model": diagnostics["model_label"],
                "shadow_model": args.model.name,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
