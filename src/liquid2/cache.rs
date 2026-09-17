//! Document cache, environment switches, and the optional Python sidecars.

use super::*;

pub(super) fn lm2_profile() -> crate::liquid::DocumentProfile {
    crate::liquid::DocumentProfile {
        kind: DocumentProfileKind::LawReviewArticle,
        confidence: 0.72,
        scores: vec![DocumentProfileScore {
            kind: DocumentProfileKind::LawReviewArticle,
            score: 0.72,
        }],
        evidence: vec!["LiquidMode2 law-review action decoder".to_owned()],
    }
}

pub(super) fn load_cached_lm2_document(source_signature: &str) -> Option<LiquidDocument> {
    let bytes = std::fs::read(lm2_cache_path(source_signature)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn save_cached_lm2_document(document: &LiquidDocument) -> Result<(), String> {
    let path = lm2_cache_path(&document.source_signature)
        .ok_or_else(|| "Could not find LiquidMode2 cache directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create LiquidMode2 cache: {error}"))?;
    }
    let bytes = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

pub(super) fn lm2_cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-cache")
            .join(format!("{source_signature}.json"))
    })
}

pub(super) fn lm2_source_signature(
    path: &Path,
    pages: &[String],
    model_label: &str,
    context_twopass_label: Option<&str>,
    pp_prior_source: Option<&str>,
    static_overlay_source: Option<&str>,
    use_pymupdf_blocks: bool,
    pp_footnote_region_membership: bool,
    marker_decoder_prior: bool,
    small_font_decoder_prior: bool,
    small_font_sequence_prior: bool,
    anchored_marginalia_flow_guard: bool,
    body_preservation_guard: bool,
    action_neutral_blocksplit: bool,
    toc_overlay: bool,
    front_matter_guard: bool,
    marginalia_preservation_guard: bool,
    d1_runtime_zerospend_overlay: bool,
    d1_runtime_zerospend_overlay_version: Option<&str>,
    d1_runtime_continuation_overlay: bool,
    d1_runtime_immediate_continuation_overlay: bool,
    d1_runtime_sandwiched_continuation_overlay: bool,
    d1_runtime_wide_sandwich_overlay: bool,
    d1_runtime_safe_numeric_note_overlay: bool,
    d1_runtime_post_wide_cue_overlay: bool,
    d1_runtime_postcue_citation_next1_overlay: bool,
    d1_runtime_near8_cue_overlay: bool,
    d1_runtime_wide_divider_guard_overlay: bool,
    d1_runtime_geometric_zone_overlay: bool,
    d1_runtime_footer_artifact_overlay: bool,
    footnote_monotone_overlay: bool,
    footnote_carryover_overlay: bool,
    table_figure_router_overlay: bool,
    page_object_overlay: bool,
    page_object_tuned_overlay: bool,
    start_score_scale: f64,
    transition_score_scale: f64,
) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let text_hash = fnv1a64(&pages.join("\n\u{0c}\n"));
    format!(
        "{:016x}",
        fnv1a64(&format!(
            "{LM2_SCHEMA_VERSION}|assembly={LM2_ASSEMBLY_CACHE_VERSION}|model={model_label}|context_twopass={}|pp={}|static_overlay={}|pymupdf={use_pymupdf_blocks}|pp_footnote_region_membership={pp_footnote_region_membership}|marker_prior={marker_decoder_prior}|small_font_prior={small_font_decoder_prior}|small_font_sequence_prior={small_font_sequence_prior}|anchored_flow_guard={anchored_marginalia_flow_guard}|body_guard={body_preservation_guard}|blocksplit={action_neutral_blocksplit}|toc_overlay={toc_overlay}|front_matter_guard={front_matter_guard}|marginalia_preservation_guard={marginalia_preservation_guard}|d1_runtime_zerospend_overlay={d1_runtime_zerospend_overlay}|d1_runtime_zerospend_overlay_version={}|d1_runtime_continuation_overlay={d1_runtime_continuation_overlay}|d1_runtime_immediate_continuation_overlay={d1_runtime_immediate_continuation_overlay}|d1_runtime_sandwiched_continuation_overlay={d1_runtime_sandwiched_continuation_overlay}|d1_runtime_wide_sandwich_overlay={d1_runtime_wide_sandwich_overlay}|d1_runtime_safe_numeric_note_overlay={d1_runtime_safe_numeric_note_overlay}|d1_runtime_post_wide_cue_overlay={d1_runtime_post_wide_cue_overlay}|d1_runtime_postcue_citation_next1_overlay={d1_runtime_postcue_citation_next1_overlay}|d1_runtime_postcue_citation_next1_overlay_version={}|d1_runtime_near8_cue_overlay={d1_runtime_near8_cue_overlay}|d1_runtime_near8_cue_overlay_version={}|d1_runtime_wide_divider_guard_overlay={d1_runtime_wide_divider_guard_overlay}|d1_runtime_wide_divider_guard_overlay_version={}|d1_runtime_geometric_zone_overlay={d1_runtime_geometric_zone_overlay}|d1_runtime_geometric_zone_overlay_version={}|d1_runtime_footer_artifact_overlay={d1_runtime_footer_artifact_overlay}|d1_runtime_footer_artifact_overlay_version={}|footnote_monotone_overlay={footnote_monotone_overlay}|footnote_monotone_overlay_version={}|footnote_carryover_overlay={footnote_carryover_overlay}|footnote_carryover_overlay_version={}|table_figure_router_overlay={table_figure_router_overlay}|table_figure_router_overlay_version={LM2_TABLE_FIGURE_ROUTER_OVERLAY_VERSION}|page_object_overlay={page_object_overlay}|page_object_overlay_version={LM2_PAGE_OBJECT_OVERLAY_VERSION}|page_object_tuned_overlay={page_object_tuned_overlay}|page_object_tuned_overlay_version={}|start_scale={start_score_scale:.6}|transition_scale={transition_score_scale:.6}|{}|{modified}|{len}|{text_hash}",
            context_twopass_label.unwrap_or("none"),
            d1_runtime_zerospend_overlay_version.unwrap_or("none"),
            if d1_runtime_postcue_citation_next1_overlay {
                LM2_D1_RUNTIME_POSTCUE_CITATION_NEXT1_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_near8_cue_overlay {
                LM2_D1_RUNTIME_NEAR8_CUE_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_wide_divider_guard_overlay {
                LM2_D1_RUNTIME_WIDE_DIVIDER_GUARD_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_geometric_zone_overlay {
                LM2_D1_RUNTIME_GEOMETRIC_ZONE_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_footer_artifact_overlay {
                LM2_D1_RUNTIME_FOOTER_ARTIFACT_OVERLAY_VERSION
            } else {
                "none"
            },
            if footnote_monotone_overlay {
                LM2_FOOTNOTE_MONOTONE_OVERLAY_VERSION
            } else {
                "none"
            },
            if footnote_carryover_overlay {
                LM2_FOOTNOTE_CARRYOVER_OVERLAY_VERSION
            } else {
                "none"
            },
            if page_object_tuned_overlay {
                LM2_PAGE_OBJECT_TUNED_OVERLAY_VERSION
            } else {
                "none"
            },
            pp_prior_source.unwrap_or("none"),
            static_overlay_source.unwrap_or("none"),
            canonical.display()
        ))
    )
}

pub(super) fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

pub(super) fn falsey_env_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

pub(super) fn falsey_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| falsey_env_value(&value))
}

pub(super) fn lm2_table_figure_router_disabled_by_env() -> bool {
    falsey_env("LAWPDF_LM2_TABLE_FIGURE_ROUTER")
}

pub(super) fn float_env_or_default(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 10.0)
        .unwrap_or(default)
}

pub(super) fn lm2_v20_runtime_preset_enabled() -> bool {
    truthy_env("LAWPDF_LM2_V20_STACK")
        || std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
            .ok()
            .is_some_and(|value| value.eq_ignore_ascii_case("v20"))
}

pub(super) fn lm2_v20_stack_runtime_enabled() -> bool {
    lm2_v20_runtime_preset_enabled() || lm2_v25_d1_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_zerospend_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_RUNTIME_ZEROSPEND_OVERLAY") || lm2_v25_d1_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_CONTINUATION_OVERLAY")
        || lm2_v25_d1_continuation_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_immediate_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_IMMEDIATE_CONTINUATION_OVERLAY")
        || lm2_v25_d1_immediate_continuation_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_sandwiched_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_SANDWICHED_CONTINUATION_OVERLAY")
        || lm2_v25_d1_sandwiched_continuation_runtime_preset_enabled()
        || lm2_v25_d1_sandwiched_note_start_runtime_preset_enabled()
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_wide_sandwich_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_WIDE_SANDWICH_OVERLAY")
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_safe_numeric_note_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_SAFE_NUMERIC_NOTE_OVERLAY")
        || lm2_v25_d1_sandwiched_note_start_runtime_preset_enabled()
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_post_wide_cue_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_POST_WIDE_CUE_OVERLAY")
        || lm2_v25_d1_post_wide_cue_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_postcue_citation_next1_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_POSTCUE_CITATION_NEXT1_OVERLAY")
        || lm2_v25_d1_postcue_citation_next1_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_near8_cue_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_NEAR8_CUE_OVERLAY") || lm2_v25_d1_near8_cue_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_wide_divider_guard_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_WIDE_DIVIDER_GUARD_OVERLAY")
        || lm2_v25_d1_wide_divider_guard_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_geometric_zone_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_GEOMETRIC_FOOTNOTE_ZONE_OVERLAY")
        || lm2_v25_d1_geometric_zone_runtime_preset_enabled()
}

pub(super) fn lm2_d1_runtime_footer_artifact_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_FOOTER_ARTIFACT_OVERLAY")
}

pub(super) fn lm2_footnote_monotone_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FOOTNOTE_MONOTONE_OVERLAY")
}

pub(super) fn lm2_footnote_carryover_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FOOTNOTE_CARRYOVER_OVERLAY")
}

pub(super) fn lm2_open_footnote_carryover_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_OPEN_FOOTNOTE_CARRYOVER_OVERLAY")
}

pub(super) fn lm2_table_figure_router_overlay_enabled() -> bool {
    !lm2_table_figure_router_disabled_by_env()
}

pub(super) fn lm2_page_object_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_PAGE_OBJECT_OVERLAY")
}

pub(super) fn lm2_page_object_tuned_overlay_enabled() -> bool {
    if falsey_env("LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY") {
        return false;
    }
    truthy_env("LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY")
        || lm2_v25_d1_page_object_tuned_runtime_preset_enabled()
}

pub(super) fn lm2_start_score_scale() -> f64 {
    float_env_or_default(
        "LAWPDF_LM2_START_SCORE_SCALE",
        if lm2_v20_stack_runtime_enabled() {
            3.0
        } else {
            1.0
        },
    )
}

pub(super) fn lm2_transition_score_scale() -> f64 {
    float_env_or_default(
        "LAWPDF_LM2_TRANSITION_SCORE_SCALE",
        if lm2_v20_stack_runtime_enabled() {
            3.0
        } else {
            1.0
        },
    )
}

pub(super) fn lm2_marker_decoder_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_MARKER_DECODER_PRIOR")
}

pub(super) fn lm2_small_font_decoder_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_SMALL_FONT_DECODER_PRIOR")
}

pub(super) fn lm2_small_font_sequence_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_SMALL_FONT_SEQUENCE_PRIOR")
}

pub(super) fn lm2_anchored_marginalia_flow_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_ANCHORED_MARGINALIA_FLOW_GUARD")
}

pub(super) fn lm2_body_preservation_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_BODY_PRESERVATION_GUARD") || lm2_v20_stack_runtime_enabled()
}

pub(super) fn lm2_action_neutral_blocksplit_enabled() -> bool {
    truthy_env("LAWPDF_LM2_ACTION_NEUTRAL_BLOCKSPLIT") || lm2_v20_stack_runtime_enabled()
}

pub(super) fn lm2_toc_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_TOC_OVERLAY") || lm2_v20_stack_runtime_enabled()
}

pub(super) fn lm2_front_matter_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FRONT_MATTER_GUARD") || lm2_v20_stack_runtime_enabled()
}

pub(super) fn lm2_marginalia_preservation_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_MARGINALIA_PRESERVATION_GUARD") || lm2_v20_stack_runtime_enabled()
}

pub(super) fn lm2_pp_footnote_region_membership_enabled() -> bool {
    truthy_env("LAWPDF_LM2_PP_FOOTNOTE_REGION_MEMBERSHIP")
}

pub(super) fn run_lm2_pp_doclayout_sidecar(
    path: &Path,
    source_lines: &[DeepLiquidSourceLine],
    cache_key: &str,
    draft_path: &Path,
) -> Result<(), String> {
    let script_path = lm2_pp_doclayout_script_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "LM2 PP-DocLayout sidecar script is missing".to_owned())?;
    let work_dir = lm2_pp_doclayout_work_dir(cache_key)
        .ok_or_else(|| "could not find LM2 PP-DocLayout work directory".to_owned())?;
    let document_path = path.to_string_lossy().to_string();
    let work_dir_text = work_dir.to_string_lossy().to_string();
    let request = Lm2PpDoclayoutRequest {
        schema_version: "lm2-pp-doclayout-runtime-request-v1",
        document_path: &document_path,
        source_lines,
        render_scale: 2.0,
        layout_model_name: "PP-DocLayoutV3",
        work_dir: &work_dir_text,
    };
    let (request_path, response_path) = write_lm2_pp_doclayout_request(cache_key, &request)?;
    if let Some(parent) = draft_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create LM2 PP-DocLayout cache: {error}"))?;
    }
    let python_exe = lm2_pp_doclayout_python_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| PathBuf::from("python3"));
    let output = std::process::Command::new(python_exe)
        .arg(&script_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--response")
        .arg(&response_path)
        .arg("--draft-output")
        .arg(draft_path)
        .output()
        .map_err(|error| format!("could not start LM2 PP-DocLayout sidecar: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "LM2 PP-DocLayout sidecar failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let bytes = std::fs::read(&response_path)
        .map_err(|error| format!("could not read LM2 PP-DocLayout response: {error}"))?;
    let response = serde_json::from_slice::<Lm2PpDoclayoutResponse>(&bytes)
        .map_err(|error| format!("could not decode LM2 PP-DocLayout response: {error}"))?;
    if !response.warnings.is_empty() {
        eprintln!(
            "LM2 PP-DocLayout sidecar warnings: {}",
            response.warnings.join("; ")
        );
    }
    eprintln!(
        "LM2 PP-DocLayout sidecar: pages={} boxes={} draft_rows={}",
        response.page_count, response.detection_box_count, response.draft_row_count
    );
    Ok(())
}

pub(super) fn write_lm2_pp_doclayout_request(
    cache_key: &str,
    request: &Lm2PpDoclayoutRequest<'_>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = app_data_dir()
        .ok_or_else(|| "could not find app data directory for LM2 PP-DocLayout".to_owned())?
        .join("liquid2-ppdoclayout-work")
        .join(cache_key);
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("could not create LM2 PP-DocLayout work directory: {error}"))?;
    let request_path = root.join("request.json");
    let response_path = root.join("response.json");
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode LM2 PP-DocLayout request: {error}"))?;
    std::fs::write(&request_path, bytes)
        .map_err(|error| format!("could not write LM2 PP-DocLayout request: {error}"))?;
    Ok((request_path, response_path))
}

pub(super) fn lm2_pp_doclayout_cache_key(
    path: &Path,
    source_lines: &[DeepLiquidSourceLine],
) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let mut line_fingerprint = String::new();
    for line in source_lines {
        line_fingerprint.push_str(&format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{:.2}\u{1f}{:.2}\u{1f}{:.2}\u{1f}{:.2}\u{1e}",
            line.page_index,
            line.line_index,
            line.text,
            line.left,
            line.bottom,
            line.right,
            line.top
        ));
    }
    format!(
        "{:016x}",
        fnv1a64(&format!(
            "{LM2_SCHEMA_VERSION}|ppdoclayout-runtime-v1|{}|{modified}|{len}|{:016x}",
            canonical.display(),
            fnv1a64(&line_fingerprint)
        ))
    )
}

pub(super) fn lm2_pp_doclayout_draft_cache_path(cache_key: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-ppdoclayout-cache")
            .join(format!("{cache_key}.jsonl"))
    })
}

pub(super) fn lm2_pp_doclayout_work_dir(cache_key: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("liquid2-ppdoclayout-work").join(cache_key))
}

pub(super) fn lm2_pp_doclayout_script_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DOCLAYOUT_SCRIPT").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("tools/lm2_pp_doclayout_regions.py"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("tools/lm2_pp_doclayout_regions.py"));
        candidates.push(exe_dir.join("../Resources/tools/lm2_pp_doclayout_regions.py"));
    }
    candidates
}

pub(super) fn lm2_pp_doclayout_python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DOCLAYOUT_PYTHON").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(".lawpdf/ppdoclayout-venv/bin/python"));
        candidates.push(current_dir.join("research/doclayout-yolo-venv/bin/python"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("../Resources/ppdoclayout-venv/bin/python"));
        candidates.push(exe_dir.join("../Resources/.lawpdf/ppdoclayout-venv/bin/python"));
    }
    candidates.push(PathBuf::from("python3"));
    candidates
}

pub(super) fn try_apply_lm2_pymupdf_grouping(
    path: &Path,
    title: &str,
    source_signature: &str,
    source_lines: &[DeepLiquidSourceLine],
) -> Result<Option<Lm2PymupdfGroupingResponse>, String> {
    if source_lines.is_empty() {
        return Ok(None);
    }
    if let Some(cached) = load_cached_lm2_pymupdf_grouping(source_signature) {
        return Ok(Some(cached));
    }
    let script_path = lm2_pymupdf_grouping_script_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "LM2 PyMuPDF grouping sidecar script is missing".to_owned())?;
    let document_path = path.to_string_lossy().to_string();
    let request = Lm2PymupdfGroupingRequest {
        schema_version: "lm2-pymupdf-grouping-request-v1",
        source_signature,
        document_path: &document_path,
        title,
        use_detector_fallback: true,
        source_lines,
    };
    let (request_path, response_path) = write_lm2_pymupdf_grouping_request(&request)?;
    let python_exe = lm2_pymupdf_grouping_python_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| PathBuf::from("python3"));
    let output = std::process::Command::new(python_exe)
        .arg(&script_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--response")
        .arg(&response_path)
        .output()
        .map_err(|error| format!("could not start LM2 PyMuPDF grouping sidecar: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "LM2 PyMuPDF grouping sidecar failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let bytes = std::fs::read(&response_path)
        .map_err(|error| format!("could not read LM2 PyMuPDF grouping response: {error}"))?;
    let response = serde_json::from_slice::<Lm2PymupdfGroupingResponse>(&bytes)
        .map_err(|error| format!("could not decode LM2 PyMuPDF grouping response: {error}"))?;
    save_cached_lm2_pymupdf_grouping(source_signature, &bytes)?;
    if response.blocks.is_empty() {
        return Ok(None);
    }
    Ok(Some(response))
}

pub(super) fn write_lm2_pymupdf_grouping_request(
    request: &Lm2PymupdfGroupingRequest<'_>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = app_data_dir()
        .ok_or_else(|| "could not find app data directory for LM2 PyMuPDF grouping".to_owned())?
        .join("liquid2-pymupdf-work");
    std::fs::create_dir_all(&root).map_err(|error| {
        format!("could not create LM2 PyMuPDF grouping work directory: {error}")
    })?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let request_path = root.join(format!("request-{nanos}.json"));
    let response_path = root.join(format!("response-{nanos}.json"));
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode LM2 PyMuPDF grouping request: {error}"))?;
    std::fs::write(&request_path, bytes)
        .map_err(|error| format!("could not write LM2 PyMuPDF grouping request: {error}"))?;
    Ok((request_path, response_path))
}

pub(super) fn load_cached_lm2_pymupdf_grouping(
    source_signature: &str,
) -> Option<Lm2PymupdfGroupingResponse> {
    let bytes = std::fs::read(lm2_pymupdf_grouping_cache_path(source_signature)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn save_cached_lm2_pymupdf_grouping(
    source_signature: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let path = lm2_pymupdf_grouping_cache_path(source_signature)
        .ok_or_else(|| "could not find LM2 PyMuPDF grouping cache directory".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create LM2 PyMuPDF grouping cache: {error}"))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("could not write LM2 PyMuPDF grouping cache: {error}"))
}

pub(super) fn lm2_pymupdf_grouping_cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-pymupdf-cache")
            .join(format!("{source_signature}.json"))
    })
}

pub(super) fn lm2_pymupdf_grouping_script_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("tools/lm2_pymupdf_blocks.py"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("tools/lm2_pymupdf_blocks.py"));
        candidates.push(exe_dir.join("../Resources/tools/lm2_pymupdf_blocks.py"));
    }
    candidates
}

pub(super) fn lm2_pymupdf_grouping_python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("research/doclayout-yolo-venv/bin/python"));
        candidates.push(current_dir.join(".lawpdf/ppdoclayout-venv/bin/python"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("../Resources/ppdoclayout-venv/bin/python"));
        candidates.push(exe_dir.join("../Resources/.lawpdf/ppdoclayout-venv/bin/python"));
    }
    candidates.push(PathBuf::from("python3"));
    candidates
}
