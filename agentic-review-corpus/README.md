# LawPDF agentic-review corpus

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
`lm2-heuristic-fallback`. The `runtime_*` columns are therefore the
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
| 011 | CHAPTER THREE CO-GOVERNANCE AND THE FUTURE OF AI REGULATION | known_hard_case | 1163 | 9.1% |
| 037 | The Cost of Justice at the Dawn of AI | best_available_with_known_artifacts | 2775 | 6.3% |
| 042 | BEYOND INTENT: ESTABLISHING DISCRIMINATORY PURPOSE IN ALGORITHMIC RISK ASSESSMENT | known_hard_case | 1062 | 19.9% |
| 047 | ANTISEMITISM, ANTI-ZIONISM, AND TITLE VI: A GUIDE FOR THE PERPLEXED Benjamin Eidelson∗ & Deborah Hellman∗∗ | known_hard_case | 1163 | 11.7% |
| 057 | CONTRACT-WRAPPED PROPERTY Danielle D’Onfro CONTENTS CONTRACT-WRAPPED PROPERTY Danielle D’Onfro∗ | known_hard_case | 3798 | 12.1% |

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
| PC1 | 16.79% | width_vs_body (+0.209), line_width_ratio (+0.209), width_norm (+0.209) |
| PC2 | 9.69% | word_count (+0.224), char_count (+0.208), line_index (+0.199) |
| PC3 | 8.79% | prev4_toc_leader_context (+0.325), prev_line_has_dotleader (+0.325), prev4_strong_dotleader_count (+0.324) |

PCA was fit once across all 9,961 source lines,
after standardizing the exact numeric model inputs. Full loadings and excluded
constant features are in `analysis-metadata.json`.
