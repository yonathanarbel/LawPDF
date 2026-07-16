# Liquid Mode 2 environment-toggle audit

Date: 2026-07-15

Scope: every named environment variable read directly or through the `truthy_env`, `falsey_env`, or `float_env_or_default` helpers in `src/liquid2.rs` and `src/layout_roles.rs`. This is a report-only phase; no rendering branch or default was changed.

Boolean parsing is asymmetric. `truthy_env` accepts `1`, `true`, `TRUE`, `yes`, or `YES`. `falsey_env` trims and case-folds `0`, `false`, `no`, or `off`. Numeric scale values must parse as finite `f64` values in `0.0..=10.0`; invalid values fall back to the listed default. “Preset” below means `LAWPDF_LM2_RUNTIME_PRESET`; the long v25 names progressively enable the D1 overlay stack. The page-object-tuned fallback acts like that preset when the packaged native CatBoost model/library pair is unavailable unless explicitly disabled.

Recommendation meanings follow `IMPROVEMENT_PLAN.md`: **promote** means make the winning behavior unconditional and remove the losing branch after owner sign-off; **keep** means retain and document a developer/deployment knob; **settings** means replace the environment knob with an `AppSettings` choice.

| Variable | Effective default when unset | What it gates | Recommendation |
|---|---|---|---|
| `LAWPDF_EXTRACT_V2` | Off | Selects the v2 PDF line-extraction path in `layout_roles.rs`. | **keep** — extraction A/B and regression diagnosis are developer concerns until v2 has corpus-wide sign-off. |
| `LAWPDF_LM2_RUNTIME_PRESET` | Unset; page-object-tuned fallback can still activate most v25 D1 behavior when native assets are absent | Selects `v20`, table/router presets, or progressively larger v25 D1 overlay bundles, including the page-object-tuned and geometric-zone variants. | **keep** — this is the central experiment/rollback knob; document accepted preset names and eventually retire superseded names. |
| `LAWPDF_LM2_V20_STACK` | Off | Legacy truthy alias for the v20 stack; also activates fallback assets, score scale 3.0, and several guards/overlays. | **keep** — retain temporarily for reproducible old experiments; prefer the named preset in new tooling. |
| `LAWPDF_LM2_PAGE_OBJECT_TUNED_DEFAULT` | Enabled only when the packaged native CatBoost model/library pair is unavailable; falsey disables it | Controls automatic promotion of the page-object-tuned fallback preset. A truthy value does not force it on when native assets exist. | **keep** — valuable packaged-runtime rollback/fallback control; rename only in a later behavior-changing phase. |
| `LAWPDF_LM2_PROGRESSIVE_PREVIEW` | On; falsey disables | Emits an early four-page Review Mode preview for eligible longer documents before the complete result. | **promote** — make progressive preview unconditional for eligible requests after owner sign-off; the exclusions already encode unsupported cases. |
| `LAWPDF_LM2_CONTEXT_TWOPASS` | On; falsey disables | Applies the promoted context two-pass model on top of native CatBoost emissions when native inference is active. | **promote** — the code already treats this as the promoted quality path and degrades safely when its model is missing. |
| `LAWPDF_LM2_TABLE_FIGURE_ROUTER` | On; falsey disables | Routes confident table/figure lines and repeated page furniture from `Keep` to hidden/table/header roles. | **settings** — hiding tables/figures is a user-visible reading tradeoff; expose a Review Mode preference instead of an environment variable. |
| `LAWPDF_LM2_D1_RUNTIME_ZEROSPEND_OVERLAY` | Off unless a v25 D1/page-object-tuned preset is effective | Enables the zero-spend D1 corrective overlay. | **keep** — corpus-evaluation knob for a rendering heuristic. |
| `LAWPDF_LM2_D1_CONTINUATION_OVERLAY` | Off unless the exact continuation preset is selected | Enables D1 continuation recovery after marginalia anchors. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_IMMEDIATE_CONTINUATION_OVERLAY` | Off unless the exact immediate-continuation preset is selected | Recovers immediate sandwiched small-font continuations. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_SANDWICHED_CONTINUATION_OVERLAY` | Off unless a sandwiched/wide/page-object-tuned preset is effective | Recovers continuation lines surrounded by marginalia evidence. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_WIDE_SANDWICH_OVERLAY` | Off unless a wide/page-object-tuned preset is effective | Enables the wider-geometry sandwiched-note recovery overlay. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_SAFE_NUMERIC_NOTE_OVERLAY` | Off unless a sandwiched-note/wide/page-object-tuned preset is effective | Recovers numeric note starts that pass conservative note-geometry guards. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_POST_WIDE_CUE_OVERLAY` | Off unless a post-cue/page-object-tuned preset is effective | Recovers forward-cued lines after the wide-sandwich stage. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_POSTCUE_CITATION_NEXT1_OVERLAY` | Off unless the matching later v25/page-object-tuned preset is effective | Recovers a citation line immediately before marginalia. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_NEAR8_CUE_OVERLAY` | Off unless the near8/page-object-tuned preset is effective | Uses nearby marginalia density within an eight-line window as recovery evidence. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_WIDE_DIVIDER_GUARD_OVERLAY` | Off unless the wide-divider/page-object-tuned preset is effective | Recovers guarded small lower-page lines below a wide divider. | **keep** — experimental heuristic switch. |
| `LAWPDF_LM2_D1_GEOMETRIC_FOOTNOTE_ZONE_OVERLAY` | Off unless a geometric-zone preset is selected | Enables the geometric lower-page footnote-zone overlay. | **keep** — experimental heuristic switch with known layout sensitivity. |
| `LAWPDF_LM2_D1_FOOTER_ARTIFACT_OVERLAY` | Off | Hides recognized repository/production footer artifacts. | **keep** — evaluate false-positive risk before promotion. |
| `LAWPDF_LM2_FOOTNOTE_MONOTONE_OVERLAY` | Off | Recovers monotone-numbered footnote runs and gaps. | **keep** — experimental footnote-sequence heuristic. |
| `LAWPDF_LM2_FOOTNOTE_CARRYOVER_OVERLAY` | Off | Carries an open footnote sequence across page boundaries. | **keep** — experimental cross-page heuristic. |
| `LAWPDF_LM2_OPEN_FOOTNOTE_CARRYOVER_OVERLAY` | Off | Enables the separate open-footnote carryover pass used during decoding. | **keep** — experimental cross-page heuristic; document its distinction from the prior row. |
| `LAWPDF_LM2_PAGE_OBJECT_OVERLAY` | Off | Enables the original page-object overlay when the native no-stack path is not active. | **keep** — retain for comparison with the tuned replacement until the older overlay is formally retired. |
| `LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY` | On only through the tuned preset/fallback; falsey forces off and truthy forces on | Enables the tuned page-object overlay. | **keep** — current rollout/rollback control for a large rendering change. |
| `LAWPDF_LM2_MARKER_DECODER_PRIOR` | Off | Adds marker-shape decoder priors when native CatBoost is not active. | **keep** — low-level model-ablation knob. |
| `LAWPDF_LM2_SMALL_FONT_DECODER_PRIOR` | Off | Adds small-font decoder priors on the non-native path. | **keep** — low-level model-ablation knob. |
| `LAWPDF_LM2_SMALL_FONT_SEQUENCE_PRIOR` | Off | Adds sequence-aware small-font priors on the non-native path. | **keep** — low-level model-ablation knob. |
| `LAWPDF_LM2_ANCHORED_MARGINALIA_FLOW_GUARD` | Off | Guards marginalia flow against unanchored starts/continuations on the non-native path. | **keep** — low-level decoder-ablation knob. |
| `LAWPDF_LM2_BODY_PRESERVATION_GUARD` | Off unless the v20/v25 D1 stack is effective | Protects likely body prose from corrective overlays on the non-native path. | **keep** — coupled to legacy/preset stack evaluation. |
| `LAWPDF_LM2_ACTION_NEUTRAL_BLOCKSPLIT` | Off unless the v20/v25 D1 stack is effective | Enables action-neutral paragraph/block splitting. | **keep** — output-shaping heuristic still coupled to preset evaluation. |
| `LAWPDF_LM2_TOC_OVERLAY` | Off unless the v20/v25 D1 stack is effective | Detects and hides table-of-contents structure in Review Mode. | **keep** — corpus-sensitive rendering heuristic. |
| `LAWPDF_LM2_FRONT_MATTER_GUARD` | Off unless the v20/v25 D1 stack is effective | Demotes/hides front-matter boilerplate and mastheads. | **keep** — corpus-sensitive rendering heuristic. |
| `LAWPDF_LM2_MARGINALIA_PRESERVATION_GUARD` | Off unless the v20/v25 D1 stack is effective | Prevents later normalization from losing predicted marginalia. | **keep** — decoder/normalizer ablation knob. |
| `LAWPDF_LM2_PP_FOOTNOTE_REGION_MEMBERSHIP` | Off | Uses PP-DocLayout region membership on the non-native path. | **keep** — research integration knob requiring an external sidecar. |
| `LAWPDF_LM2_START_SCORE_SCALE` | `3.0` with an effective v20/v25 D1 stack, otherwise `1.0` | Scales decoder start-state scores; accepts finite values from 0 through 10. | **keep** — numeric model-tuning knob, not a user preference. |
| `LAWPDF_LM2_TRANSITION_SCORE_SCALE` | `3.0` with an effective v20/v25 D1 stack, otherwise `1.0` | Scales decoder transition scores; accepts finite values from 0 through 10. | **keep** — numeric model-tuning knob, not a user preference. |
| `LAWPDF_LM2_NATIVE_CATBOOST_MODEL` | First verified packaged native-model candidate under `LAWPDF_MODEL_DIR`/the executable layout | Prepends an explicit native CatBoost model path to runtime asset candidates. | **keep** — deployment/testing override; never expose arbitrary executable asset paths in settings. |
| `LAWPDF_LM2_NATIVE_CATBOOST_LIB` | First packaged platform CatBoost library candidate | Prepends an explicit native CatBoost dynamic-library path. | **keep** — deployment/testing override. |
| `LAWPDF_LM2_CONTEXT_TWOPASS_MODEL` | First verified packaged context-model candidate | Prepends an explicit context two-pass model path. | **keep** — deployment/model-validation override even if the feature gate is promoted. |
| `LAWPDF_LM2_NUMERIC_CATBOOST_MODEL` | None; with an effective v20/v25 stack and no native model, use the packaged numeric fallback if present | Overrides the numeric CatBoost JSON model and contributes to its runtime cache label. | **keep** — fallback-model evaluation/deployment override. |
| `LAWPDF_LM2_A55_OVERLAY` | None; with an effective v20/v25 stack, use the packaged A55 overlay if present | Overrides the static A55 front-stack overlay rows on the non-native path. | **keep** — research/fallback asset override. |
| `LAWPDF_LM2_D3_OVERLAY` | None; with an effective v20/v25 stack, use the packaged D3 overlay if present | Overrides the static D3 front-matter-region overlay rows on the non-native path. | **keep** — research/fallback asset override. |
| `LAWPDF_LM2_PP_DRAFTS` | None | Loads PP draft JSONL rows as guarded per-line priors. | **keep** — explicit research-data input, not a production setting. |
| `LAWPDF_LM2_PP_DOCLAYOUT_SCRIPT` | Repository/executable-relative `tools/lm2_pp_doclayout_regions.py` candidates | Prepends an explicit PP-DocLayout sidecar script path. | **keep** — developer sidecar override. |
| `LAWPDF_LM2_PP_DOCLAYOUT_PYTHON` | Repository/resource venv candidates, then `python3` | Prepends an explicit Python interpreter for the PP-DocLayout sidecar. | **keep** — developer/runtime packaging override. |
| `LAWPDF_MODEL_DIR` | Executable-relative packaged model locations (including macOS `Resources`) | Supplies the root used before executable-relative candidates for all LM2 runtime assets. | **keep** — supported deployment override established by Phase 3. |

## Summary

- **Promote after owner sign-off:** `LAWPDF_LM2_PROGRESSIVE_PREVIEW`, `LAWPDF_LM2_CONTEXT_TWOPASS`.
- **Move to `AppSettings`:** `LAWPDF_LM2_TABLE_FIGURE_ROUTER`, because hiding table/figure material is visibly user-facing.
- **Keep and document:** the remaining 43 variables. They are model/runtime paths, research inputs, composite rollout controls, heuristic ablations, or numeric decoder tuning knobs.

This report deliberately makes no recommendation-by-implementation change. Any promotion or settings migration changes rendered document output or runtime behavior and therefore requires a separate owner-approved phase.
