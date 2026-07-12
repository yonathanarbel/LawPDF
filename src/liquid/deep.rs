use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::app_data_dir;

use super::model::{
    DeepLiquidConfig, DeepLiquidSourceLine, DocumentProfileKind, LiquidBlock, LiquidBlockRole,
};

const DEEP_LIQUID_REQUEST_SCHEMA_VERSION: u32 = 1;
#[derive(Debug, Serialize)]
struct DeepLiquidRequest<'a> {
    schema_version: u32,
    source_signature: &'a str,
    document_path: &'a str,
    title: &'a str,
    profile: DocumentProfileKind,
    lines: &'a [DeepLiquidSourceLine],
}

#[derive(Debug, Deserialize)]
struct DeepLiquidPlan {
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
    blocks: Vec<DeepLiquidBlockPlan>,
}

#[derive(Debug, Deserialize)]
struct DeepLiquidBlockPlan {
    role: LiquidBlockRole,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    visual_break_before: bool,
    #[serde(default = "default_keep")]
    action: DeepLiquidAction,
    #[serde(default)]
    source_line_ids: Vec<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DeepLiquidAction {
    Keep,
    Remove,
}

fn default_keep() -> DeepLiquidAction {
    DeepLiquidAction::Keep
}

pub(super) struct DeepLiquidResult {
    pub blocks: Vec<LiquidBlock>,
    pub model_id: String,
}

pub(super) fn try_apply_deep_liquid(
    config: &DeepLiquidConfig,
    document_path: &str,
    title: &str,
    source_signature: &str,
    profile: DocumentProfileKind,
    source_lines: &[DeepLiquidSourceLine],
) -> Result<DeepLiquidResult, String> {
    if profile != DocumentProfileKind::LawReviewArticle {
        return Err("deep Liquid is currently limited to law-review articles".to_owned());
    }
    if source_lines.is_empty() {
        return Err("deep Liquid has no canonical source lines".to_owned());
    }

    let request = DeepLiquidRequest {
        schema_version: DEEP_LIQUID_REQUEST_SCHEMA_VERSION,
        source_signature,
        document_path,
        title,
        profile,
        lines: source_lines,
    };
    let (request_path, response_path) = write_request_file(&request)?;
    run_sidecar(config, &request_path, &response_path)?;
    let response_bytes = std::fs::read(&response_path)
        .map_err(|error| format!("could not read deep Liquid response: {error}"))?;
    let plan = serde_json::from_slice::<DeepLiquidPlan>(&response_bytes)
        .map_err(|error| format!("could not decode deep Liquid response: {error}"))?;
    let model_id = plan
        .model_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.model_id.clone());
    let blocks = assemble_blocks(title, source_lines, plan)?;
    Ok(DeepLiquidResult { blocks, model_id })
}

fn write_request_file(request: &DeepLiquidRequest<'_>) -> Result<(PathBuf, PathBuf), String> {
    let root = app_data_dir()
        .ok_or_else(|| "could not find app data directory for deep Liquid".to_owned())?
        .join("liquid-deep");
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("could not create deep Liquid work directory: {error}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let request_path = root.join(format!("request-{nanos}.json"));
    let response_path = root.join(format!("response-{nanos}.json"));
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode deep Liquid request: {error}"))?;
    std::fs::write(&request_path, bytes)
        .map_err(|error| format!("could not write deep Liquid request: {error}"))?;
    Ok((request_path, response_path))
}

fn run_sidecar(
    config: &DeepLiquidConfig,
    request_path: &PathBuf,
    response_path: &PathBuf,
) -> Result<(), String> {
    if !config.script_path.exists() {
        return Err(format!(
            "deep Liquid sidecar script is missing: {}",
            config.script_path.display()
        ));
    }
    let mut command = Command::new(&config.python_exe);
    command
        .arg(&config.script_path)
        .arg("--request")
        .arg(request_path)
        .arg("--response")
        .arg(response_path)
        .arg("--model-id")
        .arg(&config.model_id);
    if let Some(model_dir) = &config.model_dir {
        command.arg("--model-dir").arg(model_dir);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start deep Liquid sidecar: {error}"))?;
    let timeout = Duration::from_secs(config.timeout_secs.max(1).min(3600));
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not poll deep Liquid sidecar: {error}"))?
        {
            if status.success() {
                return Ok(());
            }
            return Err(format!("deep Liquid sidecar exited with {status}"));
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "deep Liquid sidecar timed out after {}s",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assemble_blocks(
    title: &str,
    source_lines: &[DeepLiquidSourceLine],
    plan: DeepLiquidPlan,
) -> Result<Vec<LiquidBlock>, String> {
    if plan.blocks.is_empty() {
        return Err("deep Liquid response contained no blocks".to_owned());
    }
    if let Some(confidence) = plan.confidence
        && confidence.is_finite()
        && confidence < 0.20
    {
        return Err(format!(
            "deep Liquid document confidence too low: {confidence:.3}"
        ));
    }

    let by_id = source_lines
        .iter()
        .map(|line| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut blocks = Vec::new();
    let mut seen_visible = HashSet::new();
    let mut non_noise_text_blocks = 0usize;

    for block in plan.blocks {
        if block.action == DeepLiquidAction::Remove {
            validate_source_ids(&block.source_line_ids, &by_id)?;
            continue;
        }
        if block.source_line_ids.is_empty() {
            return Err("deep Liquid keep block had no source_line_ids".to_owned());
        }
        if let Some(confidence) = block.confidence
            && confidence.is_finite()
            && confidence < 0.05
        {
            return Err(format!(
                "deep Liquid block confidence too low: {confidence:.3}"
            ));
        }
        let lines = validate_source_ids(&block.source_line_ids, &by_id)?;
        for line in &lines {
            if !matches!(
                block.role,
                LiquidBlockRole::Noise | LiquidBlockRole::Header | LiquidBlockRole::Footer
            ) && !seen_visible.insert(line.id.as_str())
            {
                return Err(format!(
                    "deep Liquid reused source line {} in visible blocks",
                    line.id
                ));
            }
        }
        let text = lines
            .iter()
            .map(|line| line.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        if !matches!(
            block.role,
            LiquidBlockRole::Noise
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Metadata
                | LiquidBlockRole::Title
                | LiquidBlockRole::SectionBreak
        ) {
            non_noise_text_blocks += 1;
        }
        if block.visual_break_before && !blocks.is_empty() {
            push_section_break_if_needed(&mut blocks);
        }
        blocks.push(LiquidBlock {
            role: block.role,
            text,
            label: block
                .label
                .as_deref()
                .filter(|label| !label.trim().is_empty())
                .map(str::to_owned),
        });
    }

    if blocks.is_empty() {
        return Err("deep Liquid produced no displayable blocks".to_owned());
    }
    if blocks
        .first()
        .is_none_or(|block| block.role != LiquidBlockRole::Title)
    {
        blocks.insert(
            0,
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: if title.trim().is_empty() {
                    "Untitled Document".to_owned()
                } else {
                    title.trim().to_owned()
                },
                label: None,
            },
        );
    }
    if non_noise_text_blocks == 0 {
        return Err("deep Liquid produced no substantive body blocks".to_owned());
    }
    Ok(blocks)
}

fn validate_source_ids<'a>(
    source_line_ids: &[String],
    by_id: &'a HashMap<&str, &'a DeepLiquidSourceLine>,
) -> Result<Vec<&'a DeepLiquidSourceLine>, String> {
    let mut lines = Vec::with_capacity(source_line_ids.len());
    for id in source_line_ids {
        let line = by_id
            .get(id.as_str())
            .copied()
            .ok_or_else(|| format!("deep Liquid referenced unknown source line {id}"))?;
        lines.push(line);
    }
    Ok(lines)
}

fn push_section_break_if_needed(blocks: &mut Vec<LiquidBlock>) {
    if blocks
        .last()
        .is_some_and(|block| block.role != LiquidBlockRole::SectionBreak)
    {
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::SectionBreak,
            text: String::new(),
            label: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_line(
        id: &str,
        page_index: usize,
        line_index: usize,
        text: &str,
    ) -> DeepLiquidSourceLine {
        DeepLiquidSourceLine {
            id: id.to_owned(),
            page_index,
            page_width: 1.0,
            page_height: 1.0,
            line_index,
            text: text.to_owned(),
            left: 0.0,
            bottom: 0.0,
            right: 1.0,
            top: 1.0,
            page_index_norm: 0.0,
            lines_from_doc_start: line_index,
            left_margin_ratio: 0.0,
            right_margin_ratio: 0.0,
            indent_both: 0.0,
            margin_symmetry: 1.0,
            line_width_ratio: 0.0,
            indent_vs_body: 0.0,
            width_vs_body: 1.0,
            front_matter_zone: false,
            margin_centered: false,
            is_block_indented: false,
            prev_line_indented: false,
            font_height: 1.0,
            font_ratio_page: 1.0,
            font_ratio_page_ref: 1.0,
            font_ratio_doc: 1.0,
            doc_font_body_z: 0.0,
            doc_font_footnote_z: 0.0,
            doc_font_body_size: 0.0,
            doc_font_footnote_size: 0.0,
            doc_footnote_state: false,
            doc_footnote_continuation: false,
            doc_repeated_edge_text: false,
            doc_repeated_text_count: 0,
            doc_repeated_top_edge: false,
            doc_repeated_bottom_edge: false,
            doc_repeated_numeric_pattern: false,
            doc_vertical_axis_like: false,
            doc_vertical_numeric_axis_like: false,
            doc_vertical_short_text_axis_like: false,
            page_table_column_like: false,
            segment_block_id: 0,
            segment_block_line_index: 0,
            segment_block_line_count: 1,
            segment_block_first: true,
            segment_block_last: true,
            segment_block_shape: "unknown".to_owned(),
            segment_block_toc_like: false,
            segment_block_table_like: false,
            segment_block_footnote_like: false,
            segment_block_furniture_like: false,
            page_object_image_overlap_ratio: 0.0,
            page_object_image_hit_count: 0,
            page_object_path_stroke_near_line_count: 0,
            page_object_path_stroke_density_near_line: 0.0,
            page_object_thin_horizontal_near_line_count: 0,
            page_object_thin_vertical_near_line_count: 0,
            page_object_overlaps_image_bbox: false,
            page_object_ruled_row_membership: false,
            page_object_hide_candidate: false,
            page_object_hide_candidate_guarded: false,
            page_object_path15_candidate: false,
            page_object_ruled_or_path8_candidate: false,
            line_on_ruled_divider: false,
            in_ruled_cell: false,
            ruled_row_membership_exact: false,
            dist_to_nearest_rule: 0.0,
            prev_line_has_dotleader: false,
            prev4_dotleader_count: 0,
            prev4_spaced_dotleader_count: 0,
            prev4_strong_dotleader_count: 0,
            prev4_toc_leader_context: false,
            doc_note_marker: 0,
            doc_note_marker_first_on_page: false,
            doc_note_marker_mid_sequence_page: false,
            doc_note_marker_follows_previous_page: false,
            doc_note_marker_page_delta: 0,
            bold: false,
            italic: false,
            centered: false,
            below_footnote_divider: false,
            page_has_footnote_divider: false,
            in_footnote_zone: false,
            pp_prior_role: None,
            pp_prior_label: None,
            pp_prior_score: None,
            role_hint: None,
            lv: Default::default(),
        }
    }

    #[test]
    fn assemble_blocks_uses_source_line_text() {
        let lines = vec![
            source_line("p0:l0", 0, 0, "I. Introduction"),
            source_line("p0:l1", 0, 1, "This text came from the PDF."),
        ];
        let plan = DeepLiquidPlan {
            model_id: Some("test-model".to_owned()),
            confidence: Some(0.9),
            blocks: vec![
                DeepLiquidBlockPlan {
                    role: LiquidBlockRole::Heading,
                    label: None,
                    visual_break_before: false,
                    action: DeepLiquidAction::Keep,
                    source_line_ids: vec!["p0:l0".to_owned()],
                    confidence: Some(0.9),
                },
                DeepLiquidBlockPlan {
                    role: LiquidBlockRole::Paragraph,
                    label: None,
                    visual_break_before: false,
                    action: DeepLiquidAction::Keep,
                    source_line_ids: vec!["p0:l1".to_owned()],
                    confidence: Some(0.9),
                },
            ],
        };

        let blocks = assemble_blocks("Fallback Title", &lines, plan).expect("valid plan");

        assert_eq!(blocks[0].role, LiquidBlockRole::Title);
        assert_eq!(blocks[1].text, "I. Introduction");
        assert_eq!(blocks[2].text, "This text came from the PDF.");
    }

    #[test]
    fn assemble_blocks_rejects_unknown_source_ids() {
        let lines = vec![source_line("p0:l0", 0, 0, "Known line")];
        let plan = DeepLiquidPlan {
            model_id: None,
            confidence: Some(0.9),
            blocks: vec![DeepLiquidBlockPlan {
                role: LiquidBlockRole::Paragraph,
                label: None,
                visual_break_before: false,
                action: DeepLiquidAction::Keep,
                source_line_ids: vec!["p0:l99".to_owned()],
                confidence: Some(0.9),
            }],
        };

        let error = assemble_blocks("Title", &lines, plan).expect_err("invalid plan");

        assert!(error.contains("unknown source line"));
    }
}
