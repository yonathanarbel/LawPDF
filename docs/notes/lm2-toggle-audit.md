# Liquid Mode 2 environment-toggle audit

Scope: every unique environment variable read through `std::env::var`,
`std::env::var_os`, or the `truthy_env` / `falsey_env` /
`float_env_or_default` wrappers in `src/liquid2.rs` and
`src/layout_roles.rs`. “On” accepts `1`, `true`, or `yes`; “off” accepts
`0`, `false`, `no`, or `off`.

Recommendations are deliberately report-only:

- **promote**: the feature is already the normal default; after owner approval,
  make that path unconditional and remove the losing branch.
- **keep**: retain as a developer, deployment, model-selection, or recovery knob
  and document it with the associated experiment/runtime assets.
- **settings**: replace the environment variable with an `AppSettings` choice
  because it changes a user-visible product mode.

| Name | Current default | What it gates | Recommendation |
|---|---|---|---|
| `LAWPDF_EXTRACT_V2` | Off | V2 layout-role extraction in `layout_roles.rs`. | **settings** — this changes the extraction mode users experience. |
| `LAWPDF_LM2_RUNTIME_PRESET` | Unset; page-object-tuned fallback may still activate when the native model is absent. | Named V20/V25 experiment bundles and their cumulative overlay stack. | **keep** — central developer experiment selector. |
| `LAWPDF_LM2_PAGE_OBJECT_TUNED_DEFAULT` | On when the default native CatBoost asset is unavailable; explicit false disables it. | Page-object-tuned fallback preset. | **keep** — recovery/deployment control tied to model availability. |
| `LAWPDF_LM2_PROGRESSIVE_PREVIEW` | On; explicit false disables it. | Four-page progressive Review Mode preview before the full document completes. | **promote** — established user-visible default; remove the hidden kill switch after sign-off. |
| `LAWPDF_LM2_TABLE_FIGURE_ROUTER` | On; explicit false disables it. | Table/figure routing overlay. | **promote** — already the unconditional product default in practice. |
| `LAWPDF_LM2_CONTEXT_TWOPASS` | On; explicit false disables it. | Contextual two-pass model application. | **promote** — packaged model path is the normal runtime path; retain failure reporting instead of a hidden toggle. |
| `LAWPDF_LM2_V20_STACK` | Off. | Enables the legacy V20 stack and defaults several guards/overlays and score scales. | **keep** — comparison/debug preset. |
| `LAWPDF_LM2_D1_RUNTIME_ZEROSPEND_OVERLAY` | Off unless selected by a D1 preset. | D1 zero-spend overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_CONTINUATION_OVERLAY` | Off unless selected by a matching preset. | D1 continuation overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_IMMEDIATE_CONTINUATION_OVERLAY` | Off unless selected by a matching preset. | Immediate-continuation overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_SANDWICHED_CONTINUATION_OVERLAY` | Off unless selected by a matching preset. | Sandwiched-continuation/note-start overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_WIDE_SANDWICH_OVERLAY` | Off unless selected by a matching preset. | Wide-sandwich note overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_SAFE_NUMERIC_NOTE_OVERLAY` | Off unless selected by a matching preset. | Conservative numeric-note detection overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_POST_WIDE_CUE_OVERLAY` | Off unless selected by a matching preset. | Post-wide-cue note overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_POSTCUE_CITATION_NEXT1_OVERLAY` | Off unless selected by a matching preset. | Citation-next-line post-cue overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_NEAR8_CUE_OVERLAY` | Off unless selected by a matching preset. | Near-eight-lines cue overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_WIDE_DIVIDER_GUARD_OVERLAY` | Off unless selected by a matching preset. | Wide-divider false-positive guard. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_GEOMETRIC_FOOTNOTE_ZONE_OVERLAY` | Off unless selected by a matching preset. | Geometric footnote-zone overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_D1_FOOTER_ARTIFACT_OVERLAY` | Off. | Footer-artifact correction overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_FOOTNOTE_MONOTONE_OVERLAY` | Off. | Monotone footnote-sequence repair. | **keep** — experiment knob. |
| `LAWPDF_LM2_FOOTNOTE_CARRYOVER_OVERLAY` | Off. | Footnote continuation across page/region boundaries. | **keep** — experiment knob. |
| `LAWPDF_LM2_OPEN_FOOTNOTE_CARRYOVER_OVERLAY` | Off. | Open-ended footnote carryover. | **keep** — experiment knob. |
| `LAWPDF_LM2_PAGE_OBJECT_OVERLAY` | Off. | Base page-object overlay. | **keep** — experiment knob. |
| `LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY` | Follows the page-object-tuned preset/default; explicit false wins. | Tuned page-object overlay. | **keep** — model-dependent recovery/experiment control. |
| `LAWPDF_LM2_MARKER_DECODER_PRIOR` | Off. | Marker prior in sequence decoding. | **keep** — decoder experiment knob. |
| `LAWPDF_LM2_SMALL_FONT_DECODER_PRIOR` | Off. | Small-font decoder prior. | **keep** — decoder experiment knob. |
| `LAWPDF_LM2_SMALL_FONT_SEQUENCE_PRIOR` | Off. | Small-font sequence prior. | **keep** — decoder experiment knob. |
| `LAWPDF_LM2_ANCHORED_MARGINALIA_FLOW_GUARD` | Off. | Anchored-marginalia flow guard. | **keep** — heuristic experiment knob. |
| `LAWPDF_LM2_BODY_PRESERVATION_GUARD` | Off unless a V20/D1 stack preset is active. | Protects body prose from structural reclassification. | **keep** — preset-coupled heuristic. |
| `LAWPDF_LM2_ACTION_NEUTRAL_BLOCKSPLIT` | Off unless a V20/D1 stack preset is active. | Action-neutral block splitting. | **keep** — preset-coupled heuristic. |
| `LAWPDF_LM2_TOC_OVERLAY` | Off unless a V20/D1 stack preset is active. | Table-of-contents correction overlay. | **keep** — preset-coupled heuristic. |
| `LAWPDF_LM2_FRONT_MATTER_GUARD` | Off unless a V20/D1 stack preset is active. | Front-matter classification guard. | **keep** — preset-coupled heuristic. |
| `LAWPDF_LM2_MARGINALIA_PRESERVATION_GUARD` | Off unless a V20/D1 stack preset is active. | Preserves marginalia classifications. | **keep** — preset-coupled heuristic. |
| `LAWPDF_LM2_PP_FOOTNOTE_REGION_MEMBERSHIP` | Off. | PP-doclayout footnote-region membership signal. | **keep** — development/model integration knob. |
| `LAWPDF_LM2_START_SCORE_SCALE` | `3.0` with the V20/D1 stack, otherwise `1.0`; accepted range 0–10. | Sequence-decoder start-score multiplier. | **keep** — numeric tuning knob. |
| `LAWPDF_LM2_TRANSITION_SCORE_SCALE` | `3.0` with the V20/D1 stack, otherwise `1.0`; accepted range 0–10. | Sequence-decoder transition-score multiplier. | **keep** — numeric tuning knob. |
| `LAWPDF_LM2_NATIVE_CATBOOST_MODEL` | Unset; packaged runtime candidates are tried. | Override path for the native CatBoost model. | **keep** — deployment/test asset override. |
| `LAWPDF_LM2_NATIVE_CATBOOST_LIB` | Unset; packaged runtime candidates are tried. | Override path for the native CatBoost library. | **keep** — platform deployment override. |
| `LAWPDF_LM2_NUMERIC_CATBOOST_MODEL` | Unset; a packaged V20/D1 asset may be selected by the active preset. | Numeric CatBoost model path; its string also contributes to the runtime signature. | **keep** — model-development override. |
| `LAWPDF_LM2_CONTEXT_TWOPASS_MODEL` | Unset; packaged runtime candidates are tried. | Context two-pass model path override. | **keep** — model-development override. |
| `LAWPDF_LM2_A55_OVERLAY` | Unset; preset-packaged overlay may be used. | A55 static front-matter overlay file. | **keep** — experiment asset override. |
| `LAWPDF_LM2_D3_OVERLAY` | Unset; preset-packaged overlay may be used. | D3 static front-matter overlay file. | **keep** — experiment asset override. |
| `LAWPDF_LM2_PP_DRAFTS` | Unset; PP priors are disabled. | PP draft/prior input file. | **keep** — offline/model-development input. |
| `LAWPDF_MODEL_DIR` | Unset; executable-relative packaged assets are searched. | Root override for packaged LM2 runtime assets. | **keep** — deployment override. |
| `LAWPDF_LM2_PP_DOCLAYOUT_SCRIPT` | Unset; built-in candidate locations are searched. | PP-doclayout sidecar script path. | **keep** — developer sidecar override. |
| `LAWPDF_LM2_PP_DOCLAYOUT_PYTHON` | Unset; built-in Python candidates are searched. | Python executable for the PP-doclayout sidecar. | **keep** — developer environment override. |

## Recommended action order

1. Obtain output-quality sign-off before acting on any **promote** row.
2. Move `LAWPDF_EXTRACT_V2` to a named `AppSettings` field before deleting the
   environment path, so the choice is explicit and persisted.
3. Document **keep** variables in developer/model-run instructions, not end-user
   help. Path overrides should remain outside `AppSettings`.

No toggle or heuristic behavior is changed by this audit.
