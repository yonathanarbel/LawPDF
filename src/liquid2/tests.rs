use super::*;

#[test]
fn lm2_runtime_asset_candidates_follow_stable_order() {
    let model_dir = Path::new("C:/lawpdf-models");
    let exe_path = Path::new("C:/Program Files/LawPDF/lawpdf.exe");
    let candidates = lm2_runtime_asset_candidates_for(
        Some(model_dir),
        Some(exe_path),
        LM2_NATIVE_CATBOOST_RUNTIME_DIR,
        LM2_NATIVE_CATBOOST_MODEL_FILE,
    );
    assert_eq!(
        candidates,
        vec![
            model_dir
                .join("lm2-native-catboost-runtime")
                .join(LM2_NATIVE_CATBOOST_MODEL_FILE),
            Path::new("C:/Program Files/LawPDF")
                .join(LM2_NATIVE_CATBOOST_RUNTIME_DIR)
                .join(LM2_NATIVE_CATBOOST_MODEL_FILE),
            Path::new("C:/Program Files/LawPDF")
                .join("..")
                .join("Resources")
                .join(LM2_NATIVE_CATBOOST_RUNTIME_DIR)
                .join(LM2_NATIVE_CATBOOST_MODEL_FILE),
        ]
    );
}

fn lm2_test_source_line(
    id: &str,
    line_index: usize,
    text: &str,
    font_ratio_page: f32,
    centered: bool,
    role_hint: Option<LiquidBlockRole>,
) -> DeepLiquidSourceLine {
    DeepLiquidSourceLine {
        id: id.to_owned(),
        page_index: 0,
        page_width: 1.0,
        page_height: 1.0,
        line_index,
        text: text.to_owned(),
        synthetic_text_geometry: false,
        left: 0.1,
        bottom: 0.9 - (line_index as f32 * 0.02),
        right: 0.9,
        top: 0.92 - (line_index as f32 * 0.02),
        first_visual_left: 0.1,
        last_visual_right: 0.9,
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
        font_ratio_page,
        font_ratio_page_ref: 1.0,
        font_ratio_doc: font_ratio_page,
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
        centered,
        below_footnote_divider: false,
        page_has_footnote_divider: false,
        in_footnote_zone: false,
        pp_prior_role: None,
        pp_prior_label: None,
        pp_prior_score: None,
        role_hint,
        lv: Default::default(),
    }
}

#[test]
fn context_arbiter_reorders_native_scores_to_training_class_order() {
    let probabilities = lm2_context_primary_probabilities([2.0, 1.0, 0.0]);
    assert!(probabilities[1] > probabilities[2]);
    assert!(probabilities[2] > probabilities[0]);
    assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn context_arbiter_features_mask_neighbors_across_page_boundaries() {
    let first = lm2_test_source_line("p0:l0", 0, "First line", 1.0, false, None);
    let second = lm2_test_source_line("p0:l3", 3, "Second line", 1.0, false, None);
    let mut next_page = lm2_test_source_line("p1:l0", 0, "Next page", 1.0, false, None);
    next_page.page_index = 1;
    let decoded = vec![
        (first, Lm2Action::Keep),
        (second, Lm2Action::Keep),
        (next_page, Lm2Action::Keep),
    ];
    let probabilities = vec![[0.1, 0.8, 0.1], [0.2, 0.7, 0.1], [0.3, 0.6, 0.1]];

    let features = lm2_context_arbiter_feature_vector(&decoded, &probabilities, 1);

    assert_eq!(features.len(), LM2_CONTEXT_ARBITER_FEATURE_COUNT_V2);
    assert_eq!(&features[116..119], &[0.2, 0.7, 0.1]);
    assert_eq!(&features[119..122], &[0.0, 0.0, 0.0]);
    assert_eq!(&features[122..125], &[0.1, 0.8, 0.1]);
    assert_eq!(&features[125..128], &[0.0, 0.0, 0.0]);
    assert_eq!(&features[128..131], &[0.0, 0.0, 0.0]);
    assert_eq!(&features[131..135], &[0.0, 1.0, 0.0, 0.0]);
    assert_eq!(&features[135..139], &[0.0, 0.375, 0.0, 0.0]);
}

#[test]
fn runtime_residual_features_append_baseline_action_in_training_order() {
    let line = lm2_test_source_line("p0:l0", 0, "Line", 1.0, false, None);
    let decoded = vec![(line, Lm2Action::Keep)];
    let probabilities = vec![[0.1, 0.8, 0.1]];

    for (action, expected) in [
        (Lm2Action::HideNoise, [1.0, 0.0, 0.0]),
        (Lm2Action::Keep, [0.0, 1.0, 0.0]),
        (Lm2Action::Marginalia, [0.0, 0.0, 1.0]),
    ] {
        let features =
            lm2_context_runtime_residual_feature_vector(&decoded, &probabilities, action, 0);
        assert_eq!(features.len(), LM2_CONTEXT_RESIDUAL_FEATURE_COUNT_V3);
        assert_eq!(&features[LM2_CONTEXT_ARBITER_FEATURE_COUNT_V2..], &expected);
    }
}

#[test]
fn runtime_residual_rescue_requires_substantive_text() {
    assert!(!lm2_context_has_substantive_text("\""));
    assert!(!lm2_context_has_substantive_text("123 --"));
    assert!(lm2_context_has_substantive_text(
        "which he controlled, was more than $6,800,000"
    ));

    let calibration = Lm2ContextArbiterCalibration {
        rescue_keep_threshold: 0.9999,
        demote_to_marginalia_threshold: 1.0,
        demote_to_noise_threshold: 1.0,
        reclassify_nonkeep_threshold: 1.0,
    };
    assert_eq!(
        lm2_context_runtime_residual_policy(
            Lm2Action::HideNoise,
            [0.0, 1.0, 0.0],
            &calibration,
            "\"",
        ),
        Lm2Action::HideNoise
    );
    assert_eq!(
        lm2_context_runtime_residual_policy(
            Lm2Action::HideNoise,
            [0.0, 1.0, 0.0],
            &calibration,
            "which he controlled",
        ),
        Lm2Action::Keep
    );
    assert_eq!(
        lm2_context_runtime_residual_policy(
            Lm2Action::Keep,
            [1.0, 0.0, 0.0],
            &calibration,
            "body text",
        ),
        Lm2Action::Keep
    );
}

#[test]
fn context_arbiter_policy_is_asymmetric_about_body_text() {
    let calibration = Lm2ContextArbiterCalibration {
        rescue_keep_threshold: 0.8,
        demote_to_marginalia_threshold: 0.99,
        demote_to_noise_threshold: 0.95,
        reclassify_nonkeep_threshold: 0.995,
    };
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::Marginalia, [0.01, 0.81, 0.18], &calibration,),
        Lm2Action::Keep
    );
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::HideNoise, [0.01, 0.05, 0.94], &calibration,),
        Lm2Action::HideNoise
    );
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::HideNoise, [0.001, 0.001, 0.998], &calibration,),
        Lm2Action::Marginalia
    );
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::Keep, [0.03, 0.01, 0.96], &calibration,),
        Lm2Action::Keep
    );
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::Keep, [0.01, 0.0, 0.99], &calibration,),
        Lm2Action::Marginalia
    );
    assert_eq!(
        lm2_context_arbiter_policy(Lm2Action::Keep, [0.95, 0.04, 0.01], &calibration,),
        Lm2Action::HideNoise
    );
}

#[test]
fn context_hgb_export_nodes_follow_sklearn_branch_contract() {
    let leaf = |value| [value, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let tree = vec![[0.0, 0.0, 0.5, 0.0, 1.0, 2.0, 0.0], leaf(2.0), leaf(-1.0)];
    let model = Lm2ContextTwopassHgbModel {
        name: "fixture".to_owned(),
        baseline_prediction: vec![0.0, 0.0, 0.0],
        trees: vec![vec![tree.clone(), tree.clone(), tree]],
    };
    assert_eq!(model.raw_prediction(&[0.25]), [2.0, 2.0, 2.0]);
    assert_eq!(model.raw_prediction(&[0.75]), [-1.0, -1.0, -1.0]);
}

#[test]
fn note_head_features_normalize_protected_leading_markers() {
    assert_eq!(
        lm2_note_head_normalized_text("12 Authority."),
        Some((12, "12 Authority.".to_owned()))
    );
    assert_eq!(
        lm2_note_head_normalized_text("\u{E000}12\u{E001} Authority."),
        Some((12, "12 Authority.".to_owned()))
    );
    assert_eq!(lm2_note_head_normalized_text("12Authority."), None);
    assert_eq!(lm2_note_head_normalized_text("6.3 Heterogeneity"), None);
    assert_eq!(
        lm2_note_head_normalized_text("6. 3 U.S.C. authority"),
        Some((6, "6. 3 U.S.C. authority".to_owned()))
    );
    assert_eq!(lm2_note_head_normalized_text("\u{E000}0\u{E001} No."), None);
}

#[test]
fn note_head_sequence_features_use_adjacent_numeric_candidates() {
    let decoded = vec![
        (
            lm2_test_source_line("p0:l10", 10, "10 First.", 1.0, false, None),
            Lm2Action::Marginalia,
        ),
        (
            lm2_test_source_line("p0:l20", 20, "11 Second.", 1.0, false, None),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l30", 30, "12 Third.", 1.0, false, None),
            Lm2Action::Marginalia,
        ),
    ];
    let probabilities = vec![[0.1, 0.2, 0.7], [0.2, 0.3, 0.5], [0.3, 0.4, 0.3]];
    let features = lm2_note_head_sequence_features(&decoded, &probabilities);
    let middle = features[1].unwrap();
    assert_eq!(&middle[0..5], &[1.0, -0.05, 0.0, 0.2, 1.0]);
    assert_eq!(&middle[5..8], &[0.1, 0.2, 0.7]);
    assert_eq!(&middle[8..13], &[1.0, 0.05, 0.0, 0.2, 1.0]);
    assert_eq!(&middle[13..16], &[0.3, 0.4, 0.3]);
}

fn heading_reflow_fixture(
    first_text: &str,
    second_text: &str,
) -> (
    Vec<LiquidBlock>,
    Vec<LiquidBlockSourceLines>,
    Vec<(DeepLiquidSourceLine, Lm2Action)>,
) {
    let first = lm2_test_source_line(
        "p17:l9",
        9,
        first_text,
        1.0,
        true,
        Some(LiquidBlockRole::Heading),
    );
    let second = lm2_test_source_line(
        "p17:l10",
        10,
        second_text,
        1.0,
        true,
        Some(LiquidBlockRole::Heading),
    );
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: first_text.to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: second_text.to_owned(),
            label: None,
        },
    ];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&first, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&second, LiquidBlockRole::Heading)],
        },
    ];
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
    (blocks, sources, decoded)
}

#[test]
fn heading_continuation_reflow_unites_wrapped_numbered_heading() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture(
        "B. The Pigeonhole Perspective:",
        "Torts as Remedial Moral Liability Rules",
    );

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        "B. The Pigeonhole Perspective: Torts as Remedial Moral Liability Rules"
    );
    assert_eq!(sources[0].lines.len(), 2);
}

#[test]
fn heading_continuation_reflow_unites_mixed_model_roles() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture(
        "B. Question No. 2: May a Customer Choose a Security Procedure",
        "That May Not Be Commercially Reasonable?",
    );
    blocks[1].role = LiquidBlockRole::Paragraph;
    sources[1].lines[0].role = LiquidBlockRole::Paragraph;

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Heading);
    assert!(blocks[0].text.ends_with("Commercially Reasonable?"));
}

#[test]
fn heading_continuation_split_moves_only_leading_case_name_row() {
    let mut heading = lm2_test_source_line(
        "p45:l17",
        17,
        "E. Another Look at Party Presentation: New York State Rifle & Pistol Assâ€™n,",
        1.0,
        true,
        Some(LiquidBlockRole::Heading),
    );
    heading.segment_block_id = 4;
    let mut continuation = lm2_test_source_line(
        "p45:l18",
        18,
        "Inc. v. Bruen",
        1.0,
        true,
        Some(LiquidBlockRole::Paragraph),
    );
    continuation.segment_block_id = 5;
    let mut body = lm2_test_source_line(
        "p45:l19",
        19,
        "In Bruen, Justice Breyer argued in dissent that the text and history approach was difficult.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.segment_block_id = 6;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: heading.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", continuation.text, body.text),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&heading, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&continuation, LiquidBlockRole::Paragraph),
                line_ref(&body, LiquidBlockRole::Paragraph),
            ],
        },
    ];
    let decoded = vec![
        (heading, Lm2Action::Keep),
        (continuation, Lm2Action::Keep),
        (body, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_heading_leading_source_continuation_split(&mut blocks, &mut sources, &decoded,),
        1
    );
    assert!(blocks[0].text.ends_with("Inc. v. Bruen"));
    assert!(blocks[1].text.starts_with("In Bruen, Justice Breyer"));
    assert_eq!(sources[0].lines.len(), 2);
    assert_eq!(sources[1].lines.len(), 1);
}

#[test]
fn source_gap_reflow_restores_displaced_callout_bearing_body_row() {
    let previous = lm2_test_source_line(
        "p30:l50",
        50,
        "dominates current scholarly",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut displaced = lm2_test_source_line(
        "p30:l51",
        51,
        &format!("discussions.{CALLOUT_START}191{CALLOUT_END} In these cases, courts intervene by"),
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    displaced.doc_note_marker = 191;
    let current = lm2_test_source_line(
        "p30:l52",
        52,
        "unilaterally providing an amount that is easy to calculate.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut definition = lm2_test_source_line(
        "p30:l60",
        60,
        "191. See the accompanying discussion.",
        0.75,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    definition.in_footnote_zone = true;
    definition.doc_note_marker = 191;

    let mut displaced_ref = line_ref(&displaced, LiquidBlockRole::Marginalia);
    displaced_ref.note_markers = vec![191];
    let mut definition_ref = line_ref(&definition, LiquidBlockRole::Marginalia);
    definition_ref.note_markers = vec![191];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", previous.text, current.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: displaced.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: definition.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&previous, LiquidBlockRole::Paragraph),
                line_ref(&current, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![displaced_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![definition_ref],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (displaced, Lm2Action::Marginalia),
        (current, Lm2Action::Keep),
        (definition, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_source_gap_body_line_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains(&format!(
        "scholarly discussions.{CALLOUT_START}191{CALLOUT_END} In these cases"
    )));
    assert!(blocks[0].text.contains("by unilaterally providing"));
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        vec![50, 51, 52]
    );
    assert_eq!(sources[1].block_index, 1);
    assert_eq!(blocks[1].text, "191. See the accompanying discussion.");
}

#[test]
fn sandwiched_body_noise_reflow_restores_false_furniture_row() {
    let mut previous = lm2_test_source_line(
        "p58:l7",
        7,
        "the challenges of advancing ambitious statutory programs in modern times, the",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.segment_block_id = 3;
    let mut candidate = lm2_test_source_line(
        "p58:l8",
        8,
        "urgency of these remedial shortcomings demands—and seems to have secured—",
        1.05,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    candidate.segment_block_id = 3;
    candidate.centered = true;
    let mut next = lm2_test_source_line(
        "p58:l9",
        9,
        "Congress’s attention.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next.segment_block_id = 4;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: candidate.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: next.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&candidate, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&next, LiquidBlockRole::Paragraph)],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (candidate, Lm2Action::HideNoise),
        (next, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_sandwiched_body_noise_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.contains(
            "the urgency of these remedial shortcomings demands—and seems to have secured—Congress’s attention."
    ));
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        vec![7, 8, 9]
    );
    assert!(
        sources[0]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Paragraph)
    );
}

#[test]
fn sandwiched_body_noise_reflow_repairs_terminal_dehyphen_without_role_hint() {
    let mut previous = lm2_test_source_line(
        "p14:l20",
        20,
        "Even as Congress expanded the federal criminal code through statutes like",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_width = 612.0;
    previous.left = 74.46;
    let mut candidate = lm2_test_source_line(
        "p14:l21",
        21,
        "the Mann Act, the Volstead Act, and later the Gun Control Act and the Con\u{0002}",
        1.0,
        false,
        None,
    );
    candidate.page_width = 612.0;
    candidate.left = 57.96;
    let mut next = lm2_test_source_line(
        "p14:l22",
        22,
        "trolled Substances Act, federal criminal enforcement continued to expand.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next.page_width = 612.0;
    next.left = 57.96;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: candidate.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: next.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&candidate, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&next, LiquidBlockRole::Paragraph)],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (candidate, Lm2Action::HideNoise),
        (next, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_sandwiched_body_noise_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.contains("Controlled Substances Act"));
}

#[test]
fn sandwiched_body_noise_dehyphen_bridge_requires_body_margin_alignment() {
    let mut previous = lm2_test_source_line(
        "p14:l20",
        20,
        "Congress expanded the federal criminal code through statutes like",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_width = 612.0;
    let mut candidate = lm2_test_source_line(
        "p14:l21",
        21,
        "the Mann Act and later the Gun Control Act and the Con\u{0002}",
        1.0,
        false,
        None,
    );
    candidate.page_width = 612.0;
    candidate.left = 57.96;
    let mut next = lm2_test_source_line(
        "p14:l22",
        22,
        "trolled Substances Act continued the expansion.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next.page_width = 612.0;
    next.left = 120.0;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: candidate.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: next.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&candidate, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&next, LiquidBlockRole::Paragraph)],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (candidate, Lm2Action::HideNoise),
        (next, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_sandwiched_body_noise_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 3);
}

#[test]
fn embedded_small_font_note_suffix_is_split_from_body_paragraph() {
    let body_one = lm2_test_source_line(
        "p78:l32",
        32,
        "The argument proceeds from the cases and I intend to",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let body_two = lm2_test_source_line(
        "p78:l33",
        33,
        "return to that claim below.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut continuation = lm2_test_source_line(
        "p78:l35",
        35,
        "modality in a technical sense used by the preceding note.",
        0.78,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    continuation.font_ratio_page = 0.78;
    continuation.font_ratio_doc = 0.78;
    continuation.in_footnote_zone = true;
    let mut next_note = lm2_test_source_line(
        "p78:l36",
        36,
        "341. See the cited source.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {} {}", body_one.text, body_two.text, continuation.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: next_note.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&body_one, LiquidBlockRole::Paragraph),
                line_ref(&body_two, LiquidBlockRole::Paragraph),
                line_ref(&continuation, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&next_note, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (body_one, Lm2Action::Keep),
        (body_two, Lm2Action::Keep),
        (continuation, Lm2Action::Keep),
        (next_note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_embedded_small_font_note_continuation_split(&mut blocks, &mut sources, &decoded,),
        1
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(!blocks[0].text.contains("modality in a technical sense"));
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert!(blocks[1].text.starts_with("modality in a technical sense"));
    assert_eq!(blocks[2].text, "341. See the cited source.");
    assert_eq!(sources[1].lines[0].role, LiquidBlockRole::Marginalia);
}

#[test]
fn small_font_body_quote_with_stale_marginalia_refs_is_not_split() {
    let mut quote_one = lm2_test_source_line(
        "p67:l11",
        11,
        "To maintain Congress’ intent, the Board and Special Counsel must function",
        0.87,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    quote_one.font_ratio_page = 0.86;
    quote_one.font_ratio_doc = 0.87;
    quote_one.segment_block_shape = "body".to_owned();
    let mut quote_two = lm2_test_source_line(
        "p67:l12",
        12,
        "such that they fulfill the roles prescribed by Congress.",
        0.87,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    quote_two.font_ratio_page = 0.86;
    quote_two.font_ratio_doc = 0.87;
    quote_two.segment_block_shape = "body".to_owned();
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: format!("{} {}", quote_one.text, quote_two.text),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&quote_one, LiquidBlockRole::Marginalia),
            line_ref(&quote_two, LiquidBlockRole::Marginalia),
        ],
    }];
    let decoded = vec![
        (quote_one, Lm2Action::Marginalia),
        (quote_two, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_embedded_small_font_note_continuation_split(&mut blocks, &mut sources, &decoded,),
        0
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.contains("roles prescribed by Congress"));
}

#[test]
fn late_small_font_body_quote_is_rescued_before_note_reflow() {
    let mut quote = lm2_test_source_line(
        "p67:l11",
        11,
        "To maintain Congress' intent, the Board and Special Counsel must function.",
        0.87,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    quote.font_ratio_page = 1.0;
    quote.font_ratio_page_ref = 0.86;
    quote.font_ratio_doc = 0.87;
    quote.segment_block_shape = "body".to_owned();
    let mut note = lm2_test_source_line(
        "p67:l32",
        32,
        "408. Id.",
        0.81,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.in_footnote_zone = true;
    note.below_footnote_divider = true;
    note.doc_note_marker = 408;
    let decoded = vec![
        (quote.clone(), Lm2Action::Marginalia),
        (note.clone(), Lm2Action::Marginalia),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: quote.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&quote, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref_with_note_start(
                &note,
                LiquidBlockRole::Marginalia,
                true,
            )],
        },
    ];

    assert_eq!(
        apply_embedded_small_font_note_continuation_split(&mut blocks, &mut sources, &decoded,),
        0
    );
    assert_eq!(
        apply_above_note_body_marginalia_rescue(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn body_sized_physical_footnote_segment_is_not_rescued_as_body() {
    let mut previous_note = lm2_test_source_line(
        "p56:l46",
        46,
        "See Press Release, Padilla, Blumenthal Intro-",
        0.80,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.page_index = 56;
    previous_note.in_footnote_zone = true;
    let mut continuation = (18..=21)
        .map(|line_index| {
            let ratio = if line_index == 18 { 0.80 } else { 0.96 };
            let mut line = lm2_test_source_line(
                &format!("p57:l{line_index}"),
                line_index,
                match line_index {
                    18 => "duce Bill to Provide Victims of Abuse (Dec. 15,",
                    19 => "2025), https://www.padilla.senate.gov/newsroom/press-releases/",
                    20 => "padilla-blumenthal-introduce-bill-to-provide-victims",
                    _ => "[https://perma.cc/3MG9-9HQB].",
                },
                ratio,
                false,
                Some(LiquidBlockRole::Marginalia),
            );
            line.page_index = 57;
            line.font_ratio_page_ref = ratio;
            line.font_ratio_page = ratio;
            line.font_ratio_doc = ratio;
            line.in_footnote_zone = true;
            line.segment_block_id = 2;
            line.segment_block_shape = "footnote".to_owned();
            line.segment_block_footnote_like = true;
            line
        })
        .collect::<Vec<_>>();
    let mut next_note = lm2_test_source_line(
        "p57:l22",
        22,
        "288. H.R. 6091, 119th Cong. (2025).",
        0.80,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 57;
    next_note.in_footnote_zone = true;
    let continuation_text = continuation
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{CALLOUT_START}287{CALLOUT_END} See Press Release, Padilla, Blumenthal Intro-"
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: continuation_text,
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}288{CALLOUT_END} H.R. 6091, 119th Cong. (2025)."),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![287];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![288];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: continuation
                .iter()
                // The role sidecar can still carry an earlier Paragraph
                // decision when this rescue first runs; physical segment
                // provenance is the stable evidence.
                .map(|line| line_ref(line, LiquidBlockRole::Paragraph))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![next_ref],
        },
    ];
    let mut decoded = vec![(previous_note, Lm2Action::Marginalia)];
    decoded.extend(
        continuation
            .drain(..)
            .map(|line| (line, Lm2Action::Marginalia))
            .collect::<Vec<_>>(),
    );
    decoded.push((next_note, Lm2Action::Marginalia));

    // Geometry by itself is insufficient: without the open previous-page
    // note, ordinary body rescue must win and the late guard must abstain.
    blocks[0].text =
        format!("{CALLOUT_START}287{CALLOUT_END} See Press Release, Padilla, Blumenthal Intro.");
    assert_eq!(
        apply_above_note_body_marginalia_rescue(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        0
    );

    blocks[0].text =
        format!("{CALLOUT_START}287{CALLOUT_END} See Press Release, Padilla, Blumenthal Intro-");
    blocks[1].role = LiquidBlockRole::Marginalia;
    assert_eq!(
        apply_above_note_body_marginalia_rescue(&mut blocks, &sources, &decoded),
        0
    );

    // Model the final block-role drift directly: the block was promoted,
    // but its source refs still say Marginalia and all deep rows retain the
    // same physical footnote provenance.
    blocks[1].role = LiquidBlockRole::Paragraph;
    for line in &mut sources[1].lines {
        line.role = LiquidBlockRole::Marginalia;
    }
    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
}

#[test]
fn open_clause_physical_footnote_segment_spans_adjacent_blocks() {
    let mut previous_note = lm2_test_source_line(
        "p5:l53",
        53,
        "The viability of this alternative legal",
        0.81,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.page_index = 5;
    previous_note.in_footnote_zone = true;
    let continuation_texts = [
        "procedure for blocking federal policies on a nationwide basis really depends upon just",
        "how available nationwide class actions turn out to be in practice-not just in general, but",
        "at the outset of litigation, as well.\").",
    ];
    let continuation = continuation_texts
        .iter()
        .enumerate()
        .map(|(offset, text)| {
            let line_index = 21 + offset;
            let mut line = lm2_test_source_line(
                &format!("p6:l{line_index}"),
                line_index,
                text,
                0.81,
                false,
                Some(LiquidBlockRole::Marginalia),
            );
            line.page_index = 6;
            line.font_ratio_page_ref = 0.81;
            line.font_ratio_doc = 0.81;
            line.in_footnote_zone = true;
            line.below_footnote_divider = true;
            line.ruled_row_membership_exact = offset == 0;
            line.segment_block_id = 2;
            line.segment_block_line_index = offset;
            line.segment_block_line_count = continuation_texts.len();
            line.segment_block_shape = "footnote".to_owned();
            line.segment_block_footnote_like = true;
            line
        })
        .collect::<Vec<_>>();
    let mut next_note = lm2_test_source_line(
        "p6:l24",
        24,
        "25. The next authority.",
        0.81,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 6;
    next_note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}24{CALLOUT_END} The viability of this alternative legal"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: continuation[0].text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: continuation[1..]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}25{CALLOUT_END} The next authority."),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![24];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![25];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&continuation[0], LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: continuation[1..]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Marginalia))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![next_ref],
        },
    ];
    let mut decoded = vec![(previous_note, Lm2Action::Marginalia)];
    decoded.extend(
        continuation
            .into_iter()
            .map(|line| (line, Lm2Action::Marginalia)),
    );
    decoded.push((next_note, Lm2Action::Marginalia));

    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);

    // A sentence-closed note owns no continuation on the next page even
    // when the following segment has the same coarse physical geometry.
    blocks[0].text =
        format!("{CALLOUT_START}24{CALLOUT_END} The viability of this alternative legal.");
    blocks[1].role = LiquidBlockRole::Paragraph;
    blocks[2].role = LiquidBlockRole::Paragraph;
    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
}

#[test]
fn closed_citation_suffix_owns_columbia_cross_page_note_segment() {
    let mut previous_note = lm2_test_source_line(
        "p48:l52",
        52,
        "285. See Stein v. Buccaneers Ltd.",
        0.81,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.page_index = 48;
    previous_note.in_footnote_zone = true;
    let continuation_texts = [
        "P\u{2019}ship, 772 F.3d 698, 707 (11th Cir. 2014), acts",
        "diligently when it reviews the complete record",
        "and applies the governing standard.",
    ];
    let continuation = continuation_texts
        .iter()
        .enumerate()
        .map(|(offset, text)| {
            let line_index = 18 + offset;
            let mut line = lm2_test_source_line(
                &format!("p49:l{line_index}"),
                line_index,
                text,
                0.81,
                false,
                Some(LiquidBlockRole::Marginalia),
            );
            line.page_index = 49;
            line.font_ratio_page_ref = 0.81;
            line.font_ratio_doc = 0.81;
            line.in_footnote_zone = true;
            line.below_footnote_divider = false;
            line.ruled_row_membership_exact = offset == 0;
            line.segment_block_id = 2;
            line.segment_block_line_index = offset;
            line.segment_block_line_count = continuation_texts.len();
            line.segment_block_shape = "footnote".to_owned();
            line.segment_block_footnote_like = true;
            line
        })
        .collect::<Vec<_>>();
    let mut next_note = lm2_test_source_line(
        "p49:l21",
        21,
        "286. The next authority.",
        0.81,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 49;
    next_note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}285{CALLOUT_END} See Stein v. Buccaneers Ltd."),
            label: None,
        },
        paragraph_block(&continuation[0].text),
        paragraph_block(
            &continuation[1..]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}286{CALLOUT_END} The next authority."),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![285];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![286];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&continuation[0], LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: continuation[1..]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Marginalia))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![next_ref],
        },
    ];
    let mut decoded = vec![(previous_note, Lm2Action::Marginalia)];
    decoded.extend(
        continuation
            .into_iter()
            .map(|line| (line, Lm2Action::Marginalia)),
    );
    decoded.push((next_note, Lm2Action::Marginalia));

    assert_eq!(
        apply_final_physical_footnote_segment_guard(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);
}

#[test]
fn heading_continuation_reflow_promotes_paragraph_outline_anchor() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture(
        "C. Question No. 3: What Must the Bank Show",
        "to Prove Good Faith?",
    );
    blocks[0].role = LiquidBlockRole::Paragraph;
    sources[0].lines[0].role = LiquidBlockRole::Paragraph;

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Subheading);
}

#[test]
fn heading_continuation_reflow_demotes_prose_split_at_sentence_fragment() {
    let (mut blocks, mut sources, mut decoded) = heading_reflow_fixture(
        "Customer's Acknowledgment. Customer acknowledges",
        "that it was offered a commercially reasonable procedure.",
    );
    blocks[1].role = LiquidBlockRole::Paragraph;
    sources[1].lines[0].role = LiquidBlockRole::Paragraph;
    decoded[0].0.segment_block_id = 3;
    decoded[0].0.segment_block_line_index = 0;
    decoded[0].0.segment_block_line_count = 2;
    decoded[1].0.segment_block_id = 3;
    decoded[1].0.segment_block_line_index = 1;
    decoded[1].0.segment_block_line_count = 2;

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.contains("acknowledges that it was offered"));
}

#[test]
fn decoded_role_trusts_normal_size_structured_heading_hint() {
    let mut line = lm2_test_source_line(
        "p17:l18",
        18,
        "A. Ground Truth and Interpretation",
        0.97,
        false,
        Some(LiquidBlockRole::Heading),
    );
    line.font_ratio_doc = 0.97;

    assert_eq!(
        role_for_decoded_line(&line, Lm2Action::Keep, false),
        LiquidBlockRole::Heading
    );
    line.in_footnote_zone = true;
    assert_eq!(
        role_for_decoded_line(&line, Lm2Action::Keep, false),
        LiquidBlockRole::Paragraph
    );
}

#[test]
fn page_relative_font_inflation_does_not_promote_body_prose() {
    let mut line = lm2_test_source_line(
        "p6:l7",
        7,
        "Third, some scholars—notably Jack Balkin, Katharine Bartlett, Felipe",
        1.18,
        false,
        Some(LiquidBlockRole::Heading),
    );
    line.font_ratio_doc = 1.0;
    line.bold = false;
    line.centered = false;

    assert_eq!(
        role_for_decoded_line(&line, Lm2Action::Keep, false),
        LiquidBlockRole::Paragraph
    );
}

#[test]
fn heading_outline_split_separates_nested_markers_grouped_together() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture(
        "I. WRESTLING A SQUARE PEG",
        "A. Increasingly Artificial Governance",
    );
    blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "I. WRESTLING A SQUARE PEG");
    assert_eq!(blocks[1].text, "A. Increasingly Artificial Governance");
    assert_eq!(sources[0].block_index, 0);
    assert_eq!(sources[1].block_index, 1);
}

#[test]
fn heading_outline_split_separates_canonical_intro_from_front_matter() {
    let (mut blocks, mut sources, decoded) =
        heading_reflow_fixture("WHAT IS A TORT? Ketan Ramakrishnan", "INTRODUCTION");
    blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "WHAT IS A TORT? Ketan Ramakrishnan");
    assert_eq!(blocks[1].text, "INTRODUCTION");
}

#[test]
fn heading_outline_split_extracts_short_anchor_from_paragraph() {
    let (mut blocks, mut sources, mut decoded) = heading_reflow_fixture(
        "A. The Common-Law Right to Cancel a Wire Transfer",
        "Prior to Article 4A, the leading source of law was a federal opinion.",
    );
    blocks[0].role = LiquidBlockRole::Paragraph;
    blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);
    decoded[0].0.right = 0.45;
    decoded[1].0.right = 0.92;
    decoded[1].0.role_hint = Some(LiquidBlockRole::Paragraph);

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Subheading);
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[0].lines.len(), 1);
    assert_eq!(sources[1].lines.len(), 1);
}

#[test]
fn heading_outline_split_extracts_terse_numeric_anchor_without_width_cue() {
    let (mut blocks, mut sources, mut decoded) = heading_reflow_fixture(
        "3. Voting rules",
        "Quorum rules and voting rules operate together.",
    );
    blocks[0].role = LiquidBlockRole::Paragraph;
    blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);
    decoded[0].0.right = 0.92;
    decoded[0].0.role_hint = Some(LiquidBlockRole::Paragraph);
    decoded[1].0.right = 0.92;
    decoded[1].0.role_hint = Some(LiquidBlockRole::Paragraph);

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Subheading);
    assert_eq!(blocks[0].text, "3. Voting rules");
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
}

#[test]
fn heading_outline_split_extracts_lowercase_lettered_anchor() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture(
        "c. Coordination and Competition Across Jurisdictions",
        "Unlike bankruptcy, MDL does not completely aggregate all claims.",
    );
    blocks[0].role = LiquidBlockRole::Paragraph;
    blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Subheading);
    assert_eq!(
        blocks[0].text,
        "c. Coordination and Competition Across Jurisdictions"
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
}

#[test]
fn sandwiched_numbered_outline_recovers_heading_above_footnotes() {
    let previous = lm2_test_source_line(
        "p47:l24",
        24,
        "The preceding body paragraph ends here.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let heading = lm2_test_source_line(
        "p47:l25",
        25,
        "2. Allocation of Voting Rights.",
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p47:l26",
        26,
        "Even if claimants have a fair opportunity to participate, the allocation matters.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut blocks = [&previous, &heading, &next]
        .into_iter()
        .map(|line| LiquidBlock {
            role: line.role_hint.unwrap(),
            text: line.text.clone(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = [&previous, &heading, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| LiquidBlockSourceLines {
            block_index,
            lines: vec![line_ref(line, line.role_hint.unwrap())],
        })
        .collect::<Vec<_>>();
    sources[1].lines[0].note_markers = vec![2];

    assert_eq!(
        apply_sandwiched_numbered_outline_recovery(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Subheading);
    assert!(sources[1].lines[0].note_markers.is_empty());
}

#[test]
fn heading_outline_split_extracts_wrapped_and_internal_numeric_anchor() {
    let first = lm2_test_source_line(
        "p7:l33",
        33,
        "The preceding paragraph ends here.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let heading = lm2_test_source_line(
        "p7:l34",
        34,
        "3. Exception for variation by agreement in favor of the customer",
        1.0,
        true,
        None,
    );
    let body = lm2_test_source_line(
        "p7:l35",
        35,
        "Under the first exception, the parties may vary the rule.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: format!("{} {} {}", first.text, heading.text, body.text),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&first, LiquidBlockRole::Paragraph),
            line_ref(&heading, LiquidBlockRole::Paragraph),
            line_ref(&body, LiquidBlockRole::Paragraph),
        ],
    }];
    let decoded = vec![
        (first, Lm2Action::Keep),
        (heading, Lm2Action::Keep),
        (body, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Subheading);
    assert_eq!(
        blocks[1].text,
        "3. Exception for variation by agreement in favor of the customer"
    );
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
    assert_eq!(
        sources
            .iter()
            .map(|source| source.lines.len())
            .sum::<usize>(),
        3
    );
}

#[test]
fn heading_outline_split_keeps_lowercase_wrapped_heading_line() {
    let (mut blocks, mut sources, mut decoded) = heading_reflow_fixture(
        "1. Exception when customer chooses a security",
        "procedure that may not be commercially reasonable",
    );
    let body = lm2_test_source_line(
        "p17:l11",
        11,
        "The first exception applies when the customer agrees.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    blocks[0].role = LiquidBlockRole::Paragraph;
    blocks[0].text = format!("{} {} {}", blocks[0].text, blocks[1].text, body.text);
    blocks.truncate(1);
    let second_refs = sources.pop().unwrap().lines;
    sources[0].lines.extend(second_refs);
    sources[0]
        .lines
        .push(line_ref(&body, LiquidBlockRole::Paragraph));
    decoded.push((body, Lm2Action::Keep));

    assert_eq!(
        apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Subheading);
    assert!(
        blocks[0]
            .text
            .ends_with("procedure that may not be commercially reasonable")
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
}

#[test]
fn heading_outline_split_rejects_names_large_numbers_and_part_prose() {
    for candidate in [
        "C. Ring, Jr., discusses the drafting process in detail",
        "203. The proviso would be enforceable against Bank.",
        "Part IV evaluates the cancellation rules and provides recommendations",
        "1. Defining Co-Governance. — Broadly, co-governance describes many models",
        "I. JUSTICE AND THE COST DISEASE ................................ 25",
    ] {
        let (mut blocks, mut sources, mut decoded) =
            heading_reflow_fixture("Ordinary body prose introduces the point.", candidate);
        if candidate.starts_with("C. Ring") {
            decoded[1].0.right = 0.76;
        }
        blocks[0].role = LiquidBlockRole::Paragraph;
        blocks[0].text = format!("{} {}", blocks[0].text, blocks[1].text);
        blocks.truncate(1);
        let second_refs = sources.pop().unwrap().lines;
        sources[0].lines.extend(second_refs);

        assert_eq!(
            apply_heading_outline_splits(&mut blocks, &mut sources, &decoded),
            0,
            "false outline anchor: {candidate}"
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    }
}

#[test]
fn heading_outline_recognizes_westlaw_pagination_prefix() {
    assert_eq!(
        lm2_outline_heading_role("*759 IV. Evaluation and Recommendations"),
        Some(LiquidBlockRole::Heading)
    );
    assert_eq!(lm2_outline_heading_role("*7 IV. Not Pagination"), None);
}

#[test]
fn heading_outline_rejects_lowercase_page_wrap_fragments() {
    assert_eq!(lm2_outline_heading_role("introduction."), None);
    assert_eq!(
        lm2_outline_heading_role(
            "(2) to require subsequent purchasers to do the same. These servitudes"
        ),
        None
    );
    assert_eq!(
        lm2_outline_heading_role("Introduction"),
        Some(LiquidBlockRole::Heading)
    );
    assert_eq!(
        lm2_outline_heading_role("2. Cancellation After Acceptance"),
        Some(LiquidBlockRole::Subheading)
    );
}

#[test]
fn heading_continuation_reflow_unites_centered_all_caps_wrap() {
    let (mut blocks, mut sources, decoded) = heading_reflow_fixture("RESTRAINT", "ON ALIENATION");

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].text, "RESTRAINT ON ALIENATION");
}

#[test]
fn heading_continuation_reflow_preserves_distinct_outline_items() {
    let (mut blocks, mut sources, decoded) =
        heading_reflow_fixture("A. First Topic", "B. Second Topic");

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(sources.len(), 2);
}

#[test]
fn heading_continuation_reflow_never_crosses_pages() {
    let (mut blocks, mut sources, mut decoded) =
        heading_reflow_fixture("III. A Heading That Wraps", "Across a Page Boundary");
    decoded[1].0.page_index = 1;
    sources[1].lines[0].page_index = 1;

    assert_eq!(
        apply_heading_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
}

#[test]
fn drop_cap_overlay_prepends_visually_adjacent_initial() {
    let mut opening = lm2_test_source_line(
        "p1:l30",
        30,
        "an prisoners litigate habeas corpus cases?",
        1.0,
        false,
        Some(LiquidBlockRole::Heading),
    );
    opening.page_width = 600.0;
    opening.page_height = 800.0;
    opening.left = 158.46;
    opening.right = 474.48;
    opening.bottom = 295.36;
    opening.top = 304.95;
    opening.font_height = 10.98;

    let mut continuation = lm2_test_source_line(
        "p1:l31",
        31,
        "odyne procedural question is emergent.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    continuation.page_width = 600.0;
    continuation.page_height = 800.0;
    continuation.left = 158.46;
    continuation.right = 470.0;
    continuation.bottom = 283.36;
    continuation.top = 292.95;
    continuation.font_height = 10.98;

    let mut cap = lm2_test_source_line("p1:l47", 47, "C", 3.63, false, None);
    cap.page_width = 600.0;
    cap.page_height = 800.0;
    cap.left = 137.52;
    cap.right = 158.44;
    cap.bottom = 280.39;
    cap.top = 305.72;
    cap.font_height = 28.98;
    cap.font_ratio_doc = 2.70;

    let mut decoded = vec![
        (opening, Lm2Action::Keep),
        (continuation, Lm2Action::Keep),
        (cap, Lm2Action::Marginalia),
    ];
    assert_eq!(apply_drop_cap_recovery_overlay(&mut decoded), 1);
    assert_eq!(
        decoded[0].0.text,
        "Can prisoners litigate habeas corpus cases?"
    );
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Paragraph));
    assert_eq!(decoded[1].0.text, "odyne procedural question is emergent.");
    assert_eq!(decoded[2].1, Lm2Action::HideNoise);
}

#[test]
fn same_row_marker_fragment_demotes_false_heading() {
    let mut lead = lm2_test_source_line(
        "p15:l15",
        15,
        "In Garland v. Aleman Gonzalez,",
        1.16,
        false,
        None,
    );
    lead.page_width = 600.0;
    lead.page_height = 800.0;
    lead.left = 137.52;
    lead.right = 287.93;
    lead.bottom = 504.82;
    lead.top = 514.41;
    lead.italic = true;

    let mut continuation = lm2_test_source_line(
        "p15:l16",
        16,
        "129 the Court held that the statute applies.",
        1.12,
        false,
        None,
    );
    continuation.page_width = 600.0;
    continuation.page_height = 800.0;
    continuation.left = 287.94;
    continuation.right = 474.48;
    continuation.bottom = 504.31;
    continuation.top = 519.17;

    let mut decoded = vec![(lead, Lm2Action::Keep), (continuation, Lm2Action::Keep)];
    assert_eq!(apply_same_row_body_fragment_overlay(&mut decoded), 1);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Paragraph));
    let (_, blocks, _) = build_lm2_blocks("", &decoded);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(
        blocks[0].text,
        "In Garland v. Aleman Gonzalez, 129 the Court held that the statute applies."
    );
}

#[test]
fn consecutive_citation_shaped_note_heads_restore_128_and_129() {
    let mut body = lm2_test_source_line(
        "p15:l14",
        14,
        "against whom proceedings have been initiated.128",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 15;
    let mut decoded = vec![(body, Lm2Action::Keep)];
    for (offset, text) in [
        "128 8 U.S.C. § 1252(f)(1).",
        "129 142 S. Ct. 2057 (2022).",
        "130 See id. at 2062–63.",
    ]
    .into_iter()
    .enumerate()
    {
        let mut line = lm2_test_source_line(
            &format!("p15:l{}", 47 + offset),
            47 + offset,
            text,
            0.75,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 15;
        line.in_footnote_zone = true;
        decoded.push((line, Lm2Action::Marginalia));
    }

    let starts = note_start_line_ids(&decoded, &[]);
    assert!(starts.contains("p15:l47"));
    assert!(starts.contains("p15:l48"));
    assert!(starts.contains("p15:l49"));

    let (_, mut blocks, sources) = build_lm2_blocks("", &decoded);
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .count(),
        3
    );
    assert_eq!(
        apply_attached_terminal_callout_recovery(&mut blocks, &sources),
        1
    );
    assert!(blocks[0].text.contains("\u{E000}128\u{E001}"));
    let markers = sources
        .iter()
        .flat_map(|source| source.lines.iter())
        .flat_map(|line| line.note_markers.iter().copied())
        .collect::<HashSet<_>>();
    assert!(markers.is_superset(&HashSet::from([128, 129, 130])));
}

#[test]
fn table_cell_attached_ascii_callout_is_recovered() {
    let mut table_line = lm2_test_source_line(
        "p12:l20",
        20,
        "02/24/2025 Removal78",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    table_line.page_index = 12;
    let mut note = lm2_test_source_line(
        "p12:l41",
        41,
        "78. Brehm v. Marocco.",
        0.75,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 12;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: table_line.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![78];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&table_line, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
    ];

    assert_eq!(
        apply_attached_terminal_callout_recovery(&mut blocks, &sources),
        1
    );
    assert_eq!(
        blocks[0].text,
        format!("02/24/2025 Removal{CALLOUT_START}78{CALLOUT_END}")
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Table);
}

#[test]
fn table_cell_internal_ascii_callout_is_recovered() {
    let mut table_line = lm2_test_source_line(
        "p12:l23",
        23,
        "The commission is classified as major.80 An asterisk indicates removal.",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    table_line.page_index = 12;
    let mut note = lm2_test_source_line(
        "p12:l43",
        43,
        "80. Throughout, we use the definition of major commission.",
        0.75,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 12;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: table_line.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![80];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&table_line, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
    ];

    assert_eq!(
        apply_attached_terminal_callout_recovery(&mut blocks, &sources),
        1
    );
    assert_eq!(
        blocks[0].text,
        format!(
            "The commission is classified as major.{CALLOUT_START}80{CALLOUT_END} An asterisk indicates removal."
        )
    );
}

#[test]
fn page_sequence_recovers_inline_ascii_callouts_and_year_suffix() {
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: "Approved in 19891 and immediately applied the rule.2".to_owned(),
        label: None,
    }];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: Some("p0:l1".to_owned()),
                page_index: 0,
                line_index: 1,
                text: blocks[0].text.clone(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p0:l20".to_owned()),
                page_index: 0,
                line_index: 20,
                text: "1 First note.".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![1],
            }],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![LiquidSourceLineRef {
                id: Some("p0:l21".to_owned()),
                page_index: 0,
                line_index: 21,
                text: "2 Second note.".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![2],
            }],
        },
    ];

    assert_eq!(
        apply_page_sequence_ascii_callout_recovery(&mut blocks, &mut sources),
        1
    );
    assert_eq!(
        blocks[0].text,
        format!(
            "Approved in 1989{CALLOUT_START}1{CALLOUT_END} and immediately applied the rule.{CALLOUT_START}2{CALLOUT_END}"
        )
    );

    let mut incomplete = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: "Approved in 19891 and immediately applied the rule.2".to_owned(),
        label: None,
    }];
    let mut incomplete_sources = sources.clone();
    incomplete_sources.push(LiquidBlockSourceLines {
        block_index: 3,
        lines: vec![LiquidSourceLineRef {
            id: Some("p0:l22".to_owned()),
            page_index: 0,
            line_index: 22,
            text: "3 Third note.".to_owned(),
            role: LiquidBlockRole::Marginalia,
            note_markers: vec![3],
        }],
    });
    assert_eq!(
        apply_page_sequence_ascii_callout_recovery(&mut incomplete, &mut incomplete_sources,),
        0
    );
    assert_eq!(
        incomplete[0].text,
        "Approved in 19891 and immediately applied the rule.2"
    );
}

#[test]
fn page_sequence_recovers_flattened_two_digit_law_review_callouts() {
    fn source(
        block_index: usize,
        page_index: usize,
        line_index: usize,
        text: &str,
        role: LiquidBlockRole,
        note_markers: Vec<u16>,
    ) -> LiquidBlockSourceLines {
        LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: Some(format!("p{page_index}:l{line_index}")),
                page_index,
                line_index,
                text: text.to_owned(),
                role,
                note_markers,
            }],
        }
    }

    let body = [
        "Instruction.10",
        "agency principles.\"",
        "order is not authorized. 12",
        "customer's payment orders. 3",
        "Bank.19 Customer.20 Order.21",
        "customer. 2",
        "2 Article 4A.3",
        "proviso is enforceable.2s",
    ];
    let mut blocks = body
        .iter()
        .map(|text| LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: (*text).to_owned(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = body
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let (page, line) = match index {
                0..=3 => (0, index),
                4 => (1, 1),
                5 => (1, 2),
                6 => (1, 3),
                _ => (2, 1),
            };
            source(
                index,
                page,
                line,
                text,
                LiquidBlockRole::Paragraph,
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();
    // The split `22` artifact occupies a standalone source line before the
    // source line containing note 23, but both were assembled together.
    sources[6].lines = vec![
        LiquidSourceLineRef {
            id: Some("p1:l3".to_owned()),
            page_index: 1,
            line_index: 3,
            text: "2".to_owned(),
            role: LiquidBlockRole::Paragraph,
            note_markers: Vec::new(),
        },
        LiquidSourceLineRef {
            id: Some("p1:l4".to_owned()),
            page_index: 1,
            line_index: 4,
            text: "Article 4A.3".to_owned(),
            role: LiquidBlockRole::Paragraph,
            note_markers: Vec::new(),
        },
    ];
    for (offset, marker) in (10u16..=13).enumerate() {
        sources.push(source(
            8 + offset,
            0,
            20 + offset,
            &format!("{marker}. Note."),
            LiquidBlockRole::Marginalia,
            vec![marker],
        ));
    }
    sources.push(source(
        12,
        1,
        20,
        "19. Nineteen. 20. Twenty.",
        LiquidBlockRole::Marginalia,
        vec![19],
    ));
    sources.push(source(
        13,
        1,
        21,
        "21. Twenty-one. 22. Twenty-two.",
        LiquidBlockRole::Marginalia,
        vec![21],
    ));
    sources.push(source(
        14,
        1,
        22,
        "23. Twenty-three.",
        LiquidBlockRole::Marginalia,
        vec![23],
    ));
    sources.push(source(
        15,
        2,
        20,
        "28. Twenty-eight.",
        LiquidBlockRole::Marginalia,
        vec![28],
    ));

    let repaired = apply_page_sequence_ascii_callout_recovery(&mut blocks, &mut sources);
    assert_eq!(repaired, 8);
    for (index, marker) in (10u16..=13).enumerate() {
        assert!(
            blocks[index]
                .text
                .contains(&format!("{CALLOUT_START}{marker}{CALLOUT_END}")),
            "{}",
            blocks[index].text
        );
    }
    for marker in 19u16..=23 {
        assert!(
            blocks[4..=6].iter().any(|block| block
                .text
                .contains(&format!("{CALLOUT_START}{marker}{CALLOUT_END}"))),
            "missing marker {marker}: {:?}",
            &blocks[4..=6]
        );
    }
    assert!(blocks[6].text.starts_with("Article 4A."));
    assert!(
        blocks[7]
            .text
            .contains(&format!("{CALLOUT_START}28{CALLOUT_END}"))
    );
    assert!(
        sources
            .iter()
            .flat_map(|source| &source.lines)
            .any(|line| line.note_markers == [19, 20])
    );
    assert!(
        sources
            .iter()
            .flat_map(|source| &source.lines)
            .any(|line| line.note_markers == [21, 22])
    );
}

#[test]
fn block_page_sequence_prefers_existing_marker_over_earlier_ocr_letter() {
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "Mr. Ring's statement applies.".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("The point is in the public interest.{CALLOUT_START}8{CALLOUT_END}"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("The revision follows.{CALLOUT_START}9{CALLOUT_END}"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "8. First note.".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "9. Second note.".to_owned(),
            label: None,
        },
    ];
    let source = |block_index, line_index, text: &str, role, note_markers| LiquidBlockSourceLines {
        block_index,
        lines: vec![LiquidSourceLineRef {
            id: Some(format!("p4:l{line_index}")),
            page_index: 4,
            line_index,
            text: text.to_owned(),
            role,
            note_markers,
        }],
    };
    let mut sources = vec![
        source(
            0,
            2,
            "Mr. Ring's statement applies.",
            LiquidBlockRole::Paragraph,
            Vec::new(),
        ),
        source(
            1,
            10,
            &blocks[1].text,
            LiquidBlockRole::Paragraph,
            Vec::new(),
        ),
        source(
            2,
            12,
            &blocks[2].text,
            LiquidBlockRole::Paragraph,
            Vec::new(),
        ),
        source(
            3,
            30,
            "8. First note.",
            LiquidBlockRole::Marginalia,
            vec![8],
        ),
        source(
            4,
            31,
            "9. Second note.",
            LiquidBlockRole::Marginalia,
            vec![9],
        ),
    ];

    assert_eq!(
        apply_page_sequence_ascii_callout_recovery(&mut blocks, &mut sources),
        0
    );
    assert_eq!(blocks[0].text, "Mr. Ring's statement applies.");
}

#[test]
fn block_page_sequence_keeps_reading_order_for_flattened_markers() {
    let body = [
        "The bank does not have to show.'",
        "D. Question No. 4. Are the Rules Fair?",
        "The rule applies under the procedure. 5",
        "The parties chose either one.3 6",
    ];
    let mut blocks = body
        .iter()
        .map(|text| LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: (*text).to_owned(),
            label: None,
        })
        .chain((34u16..=36).map(|marker| LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{marker}. Note."),
            label: None,
        }))
        .collect::<Vec<_>>();
    let line_indices = [9usize, 10, 22, 31];
    let mut sources = body
        .iter()
        .enumerate()
        .map(|(index, text)| LiquidBlockSourceLines {
            block_index: index,
            lines: vec![LiquidSourceLineRef {
                id: Some(format!("p14:l{}", line_indices[index])),
                page_index: 14,
                line_index: line_indices[index],
                text: (*text).to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        })
        .collect::<Vec<_>>();
    sources.extend(
        (34u16..=36)
            .enumerate()
            .map(|(offset, marker)| LiquidBlockSourceLines {
                block_index: 4 + offset,
                lines: vec![LiquidSourceLineRef {
                    id: Some(format!("p14:l{}", 34 + offset)),
                    page_index: 14,
                    line_index: 34 + offset,
                    text: format!("{marker}. Note."),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![marker],
                }],
            }),
    );

    assert_eq!(
        apply_page_sequence_ascii_callout_recovery(&mut blocks, &mut sources),
        3
    );
    assert!(
        blocks[0]
            .text
            .ends_with(&format!("{CALLOUT_START}34{CALLOUT_END}"))
    );
    assert_eq!(blocks[1].text, body[1]);
    assert!(
        blocks[2]
            .text
            .ends_with(&format!("{CALLOUT_START}35{CALLOUT_END}"))
    );
    assert!(
        blocks[3]
            .text
            .contains(&format!("{CALLOUT_START}36{CALLOUT_END}"))
    );
}

#[test]
fn decoded_page_sequence_preempts_one_digit_callout_guessing() {
    let mut first = lm2_test_source_line(
        "p10:l4",
        4,
        "The conflict should be resolved.2 6",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    first.page_index = 10;
    // This is the real failure mode: the line model hid the dehyphenated
    // continuation even though final assembly later restored it as body.
    first.role_hint = Some(LiquidBlockRole::Noise);
    let mut second = lm2_test_source_line(
        "p10:l5",
        5,
        "The procedure was chosen.27",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    second.page_index = 10;
    let mut note_26 = lm2_test_source_line(
        "p10:l30",
        30,
        "26. First note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_26.page_index = 10;
    note_26.in_footnote_zone = true;
    let mut note_27 = lm2_test_source_line(
        "p10:l31",
        31,
        "27. Second note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_27.page_index = 10;
    note_27.in_footnote_zone = true;

    let mut decoded = vec![
        (first, Lm2Action::HideNoise),
        (second, Lm2Action::Keep),
        (note_26, Lm2Action::Marginalia),
        (note_27, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_decoded_page_sequence_callout_recovery(&mut decoded),
        2
    );
    assert!(
        decoded[0]
            .0
            .text
            .contains(&format!("{CALLOUT_START}26{CALLOUT_END}"))
    );
    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert!(
        !decoded[0]
            .0
            .text
            .contains(&format!("{CALLOUT_START}2{CALLOUT_END}"))
    );
    assert!(
        !decoded[0]
            .0
            .text
            .contains(&format!("{CALLOUT_START}6{CALLOUT_END}"))
    );
    assert!(
        decoded[1]
            .0
            .text
            .contains(&format!("{CALLOUT_START}27{CALLOUT_END}"))
    );
}

fn scanned_glyph_test_body(line_index: usize, marker: u16) -> DeepLiquidSourceLine {
    let mut line = lm2_test_source_line(
        &format!("p12:l{line_index}"),
        line_index,
        &format!("The body proposition is established.{marker} It continues."),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    line.page_index = 12;
    line.page_width = 400.0;
    line.page_height = 600.0;
    line.left = 30.0;
    line.right = 360.0;
    line
}

fn scanned_glyph_test_note(
    line_index: usize,
    left: f32,
    text: &str,
    segment_first: bool,
) -> DeepLiquidSourceLine {
    let mut line = lm2_test_source_line(
        &format!("p12:l{line_index}"),
        line_index,
        text,
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    line.page_index = 12;
    line.page_width = 400.0;
    line.page_height = 600.0;
    line.left = left;
    line.right = 360.0;
    line.font_ratio_page_ref = 0.76;
    line.in_footnote_zone = true;
    line.segment_block_footnote_like = true;
    line.segment_block_shape = "footnote".to_owned();
    line.segment_block_first = segment_first;
    line
}

#[test]
fn scanned_glyph_sequence_bridge_recovers_only_complete_exact_page_pair() {
    let mut decoded = (67..=70)
        .enumerate()
        .map(|(position, marker)| {
            (
                scanned_glyph_test_body(position + 1, marker),
                Lm2Action::Keep,
            )
        })
        .collect::<Vec<_>>();
    decoded.push((
        scanned_glyph_test_note(20, 29.5, "continuation of an earlier note", false),
        Lm2Action::Marginalia,
    ));
    for (position, text) in [
        "\"See note 12 supra.",
        "mSupra p. 270.",
        "nSEN. Doc. No. 8.",
        "\"Legis. (1941) 41 Col. L. Rev. 946.",
    ]
    .into_iter()
    .enumerate()
    {
        decoded.push((
            scanned_glyph_test_note(21 + position, 37.2, text, position != 1),
            Lm2Action::HideNoise,
        ));
    }
    decoded.push((
        scanned_glyph_test_note(25, 29.7, "continuation of note 70", false),
        Lm2Action::Marginalia,
    ));

    assert_eq!(
        scanned_glyph_body_marker_sequence(&decoded, &[0, 1, 2, 3]),
        Some(vec![67, 68, 69, 70])
    );
    assert_eq!(
        scanned_glyph_note_head_rows(
            &decoded,
            &(0..decoded.len()).collect::<Vec<_>>(),
            &[67, 68, 69, 70],
        ),
        Some(vec![5, 6, 7, 8])
    );
    assert_eq!(
        apply_scanned_glyph_note_sequence_bridge(Path::new("synthetic.pdf"), &mut decoded),
        4
    );
    for (position, marker) in (67..=70).enumerate() {
        let note = &decoded[5 + position];
        assert_eq!(note.0.doc_note_marker, marker);
        assert_eq!(note.1, Lm2Action::Marginalia);
    }
    for (position, marker) in (67..=70).enumerate() {
        assert!(
            decoded[position]
                .0
                .text
                .contains(&format!("{CALLOUT_START}{marker}{CALLOUT_END}"))
        );
    }
    assert_eq!(
        apply_decoded_page_sequence_callout_recovery(&mut decoded),
        0
    );
}

#[test]
fn scanned_glyph_sequence_bridge_rejects_mismatched_readable_heads() {
    let mut decoded = (1..=4)
        .enumerate()
        .map(|(position, marker)| {
            (
                scanned_glyph_test_body(position + 1, marker),
                Lm2Action::Keep,
            )
        })
        .collect::<Vec<_>>();
    decoded.push((
        scanned_glyph_test_note(20, 29.0, "continuation", false),
        Lm2Action::Marginalia,
    ));
    for (position, marker) in (77..=80).enumerate() {
        decoded.push((
            scanned_glyph_test_note(21 + position, 37.0, &format!("{marker}. Id."), true),
            Lm2Action::Marginalia,
        ));
    }

    assert_eq!(
        apply_scanned_glyph_note_sequence_bridge(Path::new("synthetic.pdf"), &mut decoded),
        0
    );
    assert!(decoded.iter().all(|(line, _)| line.doc_note_marker == 0));
}

#[test]
fn scanned_glyph_sequence_bridge_rejects_indented_quote_continuations() {
    let mut decoded = (20..=23)
        .enumerate()
        .map(|(position, marker)| {
            (
                scanned_glyph_test_body(position + 1, marker),
                Lm2Action::Keep,
            )
        })
        .collect::<Vec<_>>();
    decoded.push((
        scanned_glyph_test_note(20, 120.0, "ordinary note continuation", true),
        Lm2Action::Marginalia,
    ));
    for (position, text) in [
        "The quoted passage begins here",
        "and continues on the next row",
        "with the same block indentation",
        "until its final quoted sentence.",
    ]
    .into_iter()
    .enumerate()
    {
        decoded.push((
            scanned_glyph_test_note(21 + position, 132.0, text, false),
            Lm2Action::Marginalia,
        ));
    }

    assert_eq!(
        apply_scanned_glyph_note_sequence_bridge(Path::new("synthetic.pdf"), &mut decoded),
        0
    );
}

#[test]
fn partial_existing_callout_rejects_single_letter_formula_subscript() {
    let text = format!("sector C{CALLOUT_START}2{CALLOUT_END}, remains constant");
    assert_eq!(partial_existing_sentinel_range(&text, "52", 0), None);
}

#[test]
fn partial_existing_callout_rejects_parenthesized_enumeration() {
    let text = format!("first; and ({CALLOUT_START}2{CALLOUT_END}) second");
    assert_eq!(partial_existing_sentinel_range(&text, "92", 0), None);
}

#[test]
fn decoded_page_sequence_collapses_premature_one_digit_sentinels() {
    let mut first = lm2_test_source_line(
        "p10:l6",
        6,
        &format!(
            "The conflict should be resolved.{CALLOUT_START}2{CALLOUT_END} {CALLOUT_START}6{CALLOUT_END}"
        ),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    first.page_index = 10;
    let mut second = lm2_test_source_line(
        "p10:l17",
        17,
        &format!("The procedure was chosen.{CALLOUT_START}27{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    second.page_index = 10;
    let mut note_26 = lm2_test_source_line(
        "p10:l27",
        27,
        "26. First note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_26.page_index = 10;
    note_26.in_footnote_zone = true;
    let mut note_27 = lm2_test_source_line(
        "p10:l43",
        43,
        "27. Second note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_27.page_index = 10;
    note_27.in_footnote_zone = true;
    let mut decoded = vec![
        (first, Lm2Action::Keep),
        (second, Lm2Action::Keep),
        (note_26, Lm2Action::Marginalia),
        (note_27, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_decoded_page_sequence_callout_recovery(&mut decoded),
        1
    );
    assert!(
        decoded[0]
            .0
            .text
            .ends_with(&format!("{CALLOUT_START}26{CALLOUT_END}"))
    );
    assert!(
        !decoded[0]
            .0
            .text
            .contains(&format!("{CALLOUT_START}2{CALLOUT_END}"))
    );
    assert!(
        !decoded[0]
            .0
            .text
            .contains(&format!("{CALLOUT_START}6{CALLOUT_END}"))
    );
}

#[test]
fn decoded_page_sequence_prefers_real_existing_marker_over_earlier_ocr_letter() {
    let mut surname = lm2_test_source_line(
        "p4:l2",
        2,
        "Mr. Ring's statement applies.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    surname.page_index = 4;
    let mut actual_8 = lm2_test_source_line(
        "p4:l10",
        10,
        &format!("The point is in the public interest.{CALLOUT_START}8{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    actual_8.page_index = 4;
    let mut actual_9 = lm2_test_source_line(
        "p4:l12",
        12,
        &format!("The revision follows.{CALLOUT_START}9{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    actual_9.page_index = 4;
    let mut note_8 = lm2_test_source_line(
        "p4:l30",
        30,
        "8. First note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_8.page_index = 4;
    note_8.in_footnote_zone = true;
    let mut note_9 = lm2_test_source_line(
        "p4:l31",
        31,
        "9. Second note.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note_9.page_index = 4;
    note_9.in_footnote_zone = true;
    let mut decoded = vec![
        (surname, Lm2Action::Keep),
        (actual_8, Lm2Action::Keep),
        (actual_9, Lm2Action::Keep),
        (note_8, Lm2Action::Marginalia),
        (note_9, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_decoded_page_sequence_callout_recovery(&mut decoded),
        0
    );
    assert_eq!(decoded[0].0.text, "Mr. Ring's statement applies.");
}

#[test]
fn same_page_body_callout_restores_isolated_citation_note_head() {
    let mut body = lm2_test_source_line(
        "p3:l25",
        25,
        &format!("The statute applies.{CALLOUT_START}15{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 3;
    let mut real_head = lm2_test_source_line(
        "p3:l41",
        41,
        "15 28 U.S.C. § 2241.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    real_head.page_index = 3;
    real_head.in_footnote_zone = true;
    let mut reporter_citation = lm2_test_source_line(
        "p49:l41",
        41,
        "15 S. Ct. 2392 (1895).",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    reporter_citation.page_index = 49;
    reporter_citation.in_footnote_zone = true;
    let decoded = vec![
        (body, Lm2Action::Keep),
        (real_head, Lm2Action::Marginalia),
        (reporter_citation, Lm2Action::Marginalia),
    ];

    let starts = same_page_body_referenced_note_heads(&decoded);
    assert!(starts.contains("p3:l41"));
    assert!(!starts.contains("p49:l41"));
}

#[test]
fn same_page_body_callout_prefers_contextual_sentineled_note_head() {
    let mut body = lm2_test_source_line(
        "p40:l8",
        8,
        &format!("The argument continues.{CALLOUT_START}176{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 40;
    let mut false_pincite = lm2_test_source_line(
        "p40:l35",
        35,
        "176 (“[L]essons Learned”).",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    false_pincite.page_index = 40;
    false_pincite.in_footnote_zone = true;
    let mut actual = lm2_test_source_line(
        "p40:l38",
        38,
        &format!("Prior citation. {CALLOUT_START}176{CALLOUT_END} Actual note."),
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    actual.page_index = 40;
    actual.in_footnote_zone = true;
    let decoded = vec![
        (body, Lm2Action::Keep),
        (false_pincite, Lm2Action::Marginalia),
        (actual, Lm2Action::Marginalia),
    ];

    let starts = same_page_body_referenced_note_heads(&decoded);
    assert!(!starts.contains("p40:l35"));
}

#[test]
fn same_page_callout_overlay_recovers_detached_leading_and_terminal_markers() {
    let mut previous = lm2_test_source_line(
        "p17:l3",
        3,
        "tinker with Rules 23 and 81.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 17;
    previous.page_width = 612.0;
    previous.page_height = 792.0;
    previous.left = 137.52;
    previous.right = 272.03;
    previous.bottom = 646.81;
    previous.top = 661.67;
    previous.font_height = 10.98;
    let mut detached =
        lm2_test_source_line("p17:l4", 4, "137", 0.6, false, Some(LiquidBlockRole::Noise));
    detached.page_index = 17;
    detached.page_width = 612.0;
    detached.left = 271.98;
    detached.right = 283.87;
    detached.bottom = 651.16;
    detached.top = 656.83;
    detached.font_height = 6.48;
    let mut leading = lm2_test_source_line(
        "p17:l5",
        5,
        "138 his behalf.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    leading.page_index = 17;
    let mut terminal = lm2_test_source_line(
        "p17:l6",
        6,
        "The statute applies.139",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    terminal.page_index = 17;
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (detached, Lm2Action::HideNoise),
        (leading, Lm2Action::Keep),
        (terminal, Lm2Action::Keep),
    ];
    for (offset, marker) in [137, 138, 139].into_iter().enumerate() {
        let mut definition = lm2_test_source_line(
            &format!("p17:l{}", 36 + offset),
            36 + offset,
            &format!("{marker} See authority."),
            0.7,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        definition.page_index = 17;
        definition.in_footnote_zone = true;
        decoded.push((definition, Lm2Action::Marginalia));
    }

    assert_eq!(apply_same_page_body_callout_overlay(&mut decoded), 3);
    assert!(
        decoded[0]
            .0
            .text
            .ends_with(&format!("{CALLOUT_START}137{CALLOUT_END}"))
    );
    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[1].1, Lm2Action::HideNoise);
    assert!(
        decoded[2]
            .0
            .text
            .starts_with(&format!("{CALLOUT_START}138{CALLOUT_END}"))
    );
    assert!(
        decoded[3]
            .0
            .text
            .ends_with(&format!("{CALLOUT_START}139{CALLOUT_END}"))
    );
}

#[test]
fn terminal_marker_span_keeps_closing_punctuation_and_never_splits_a_char() {
    // A curly closing quote is three bytes wide; the old rewrite sliced
    // `len - digits.len()` and landed inside it.
    let curly = "as the Court held in the dissent.12\u{201D}";
    let (marker, span) = attached_terminal_ascii_marker_span(curly).expect("marker");
    assert_eq!(marker, 12);
    assert_eq!(&curly[span.clone()], "12");
    assert_eq!(
        rewrite_terminal_marker_as_callout(curly, marker, &span).as_deref(),
        Some(
            format!("as the Court held in the dissent.{CALLOUT_START}12{CALLOUT_END}\u{201D}")
                .as_str()
        )
    );

    let paren = "as the Court held in the dissent.12)";
    let (marker, span) = attached_terminal_ascii_marker_span(paren).expect("marker");
    assert_eq!(
        rewrite_terminal_marker_as_callout(paren, marker, &span).as_deref(),
        Some(format!("as the Court held in the dissent.{CALLOUT_START}12{CALLOUT_END})").as_str())
    );

    let apostrophe = "the plaintiffs\u{2019} claim fails.7\u{2019}  ";
    let (marker, span) = attached_terminal_ascii_marker_span(apostrophe).expect("marker");
    assert_eq!(marker, 7);
    assert_eq!(&apostrophe[span.clone()], "7");

    let plain = "The statute applies.139";
    let (marker, span) = attached_terminal_ascii_marker_span(plain).expect("marker");
    assert_eq!(marker, 139);
    assert_eq!(
        rewrite_terminal_marker_as_callout(plain, marker, &span).as_deref(),
        Some(format!("The statute applies.{CALLOUT_START}139{CALLOUT_END}").as_str())
    );

    assert!(attached_terminal_ascii_marker_span("no marker here").is_none());
    assert!(attached_terminal_ascii_marker_span("1942").is_none());
}

#[test]
fn leading_marker_length_counts_leading_zeros() {
    assert_eq!(
        leading_numeric_token_marker_with_len("007 text"),
        Some((7, 3))
    );
    assert_eq!(
        leading_numeric_token_marker_with_len("  12. text"),
        Some((12, 2))
    );
    assert_eq!(leading_numeric_token_marker_with_len("6.3 Heading"), None);
}

#[test]
fn same_page_callout_overlay_survives_curly_quote_after_terminal_marker() {
    let mut body = lm2_test_source_line(
        "p3:l4",
        4,
        "as the Court held in the dissent.12\u{201D}",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 3;
    let mut definition = lm2_test_source_line(
        "p3:l40",
        40,
        "12 See authority.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    definition.page_index = 3;
    definition.in_footnote_zone = true;
    let mut decoded = vec![(body, Lm2Action::Keep), (definition, Lm2Action::Marginalia)];

    assert_eq!(apply_same_page_body_callout_overlay(&mut decoded), 1);
    assert_eq!(
        decoded[0].0.text,
        format!("as the Court held in the dissent.{CALLOUT_START}12{CALLOUT_END}\u{201D}")
    );
}

#[test]
fn detached_callout_restores_dehyphenated_hidden_context() {
    let mut lead = lm2_test_source_line(
        "p22:l1",
        1,
        "courts could unquestionably incorpo-",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    lead.page_index = 22;
    let mut continuation = lm2_test_source_line(
        "p22:l2",
        2,
        "rate FRCP content analogically, by force of the All Writs Act and",
        1.0,
        false,
        Some(LiquidBlockRole::Noise),
    );
    continuation.page_index = 22;
    let mut terminal = lm2_test_source_line(
        "p22:l3",
        3,
        "§ 2243.",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    terminal.page_index = 22;
    terminal.page_width = 612.0;
    terminal.left = 137.5;
    terminal.right = 171.0;
    terminal.bottom = 647.4;
    terminal.top = 662.2;
    terminal.font_height = 10.98;
    let mut marker = lm2_test_source_line(
        "p22:l4",
        4,
        "178",
        0.6,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    marker.page_index = 22;
    marker.page_width = 612.0;
    marker.left = 171.0;
    marker.right = 182.9;
    marker.bottom = 651.7;
    marker.top = 657.4;
    marker.font_height = 6.48;
    let mut definition =
        lm2_test_source_line("p22:l25", 25, "178 See id. at 299.", 0.7, false, None);
    definition.page_index = 22;
    definition.in_footnote_zone = true;
    let mut decoded = vec![
        (lead, Lm2Action::Keep),
        (continuation, Lm2Action::HideNoise),
        (terminal, Lm2Action::Marginalia),
        (marker, Lm2Action::Marginalia),
        (definition, Lm2Action::Marginalia),
    ];

    assert_eq!(apply_same_page_body_callout_overlay(&mut decoded), 2);
    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[2].1, Lm2Action::Keep);
    assert!(
        decoded[2]
            .0
            .text
            .ends_with(&format!("{CALLOUT_START}178{CALLOUT_END}"))
    );
    let (_, blocks, _) = build_lm2_blocks("", &decoded);
    assert!(
        blocks[0]
            .text
            .contains("unquestionably incorporate FRCP content analogically")
    );
}

#[test]
fn paragraph_leading_callout_moves_back_to_sentence_end() {
    let marker = format!("{CALLOUT_START}91{CALLOUT_END}");
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "The feature dated from 1970.".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{marker} Leaning on the original litigation, the court proceeded."),
            label: None,
        },
    ];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: Some("p10:l26".to_owned()),
                page_index: 10,
                line_index: 26,
                text: "The feature dated from 1970.".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: vec![],
            }],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p10:l27".to_owned()),
                page_index: 10,
                line_index: 27,
                text: "91 Leaning on the original litigation,".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: vec![91],
            }],
        },
    ];

    assert_eq!(apply_leading_callout_backfill(&mut blocks, &sources), 1);
    assert_eq!(
        blocks[0].text,
        format!("The feature dated from 1970.{marker}")
    );
    assert_eq!(
        blocks[1].text,
        "Leaning on the original litigation, the court proceeded."
    );
}

#[test]
fn in_block_standalone_callout_replaces_plain_appended_digits() {
    let mut previous = lm2_test_source_line(
        "p24:l25",
        25,
        "Middendorf reserved that question in 1976,",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 24;
    previous.page_width = 612.0;
    previous.left = 137.5;
    previous.right = 462.6;
    previous.bottom = 381.3;
    previous.top = 396.2;
    previous.font_height = 10.98;
    let mut marker = lm2_test_source_line(
        "p24:l26",
        26,
        "198",
        0.6,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    marker.page_index = 24;
    marker.page_width = 612.0;
    marker.left = 462.66;
    marker.right = 474.55;
    marker.bottom = 385.66;
    marker.top = 391.33;
    marker.font_height = 6.48;
    let mut definition = lm2_test_source_line(
        "p24:l49",
        49,
        "198 See Middendorf v. Henry.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    definition.page_index = 24;
    definition.in_footnote_zone = true;
    // Deliberately shuffled: decoder storage order is not reading order.
    let decoded = vec![
        (marker.clone(), Lm2Action::Keep),
        (definition, Lm2Action::Marginalia),
        (previous.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: "Middendorf reserved that question in 1976, 198 following a reservation.".to_owned(),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&previous, LiquidBlockRole::Paragraph),
            line_ref(&marker, LiquidBlockRole::Paragraph),
        ],
    }];

    assert_eq!(
        apply_in_block_standalone_callout_recovery(&mut blocks, &sources, &decoded),
        1
    );
    assert!(
        blocks[0]
            .text
            .contains(&format!("1976,{CALLOUT_START}198{CALLOUT_END} following"))
    );
}

#[test]
fn in_block_standalone_callout_accepts_collapsed_multiline_bbox() {
    let mut previous = lm2_test_source_line(
        "p6:l14",
        14,
        "Employers have tools to keep workers on message.\u{201d}",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 6;
    previous.page_width = 612.0;
    previous.left = 126.0;
    previous.right = 412.0;
    previous.bottom = 396.1;
    previous.top = 422.5;
    previous.font_height = 9.2;
    let mut marker = lm2_test_source_line(
        "p6:l15",
        15,
        "26",
        0.6,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    marker.page_index = 6;
    marker.page_width = 612.0;
    marker.left = 381.0;
    marker.right = 393.0;
    marker.bottom = 401.0;
    marker.top = 409.9;
    marker.font_height = 6.47;
    let mut definition = lm2_test_source_line(
        "p6:l40",
        40,
        "26 See Roberts v. Jaycees.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    definition.page_index = 6;
    definition.in_footnote_zone = true;
    let decoded = vec![
        (marker.clone(), Lm2Action::Keep),
        (definition.clone(), Lm2Action::Marginalia),
        (previous.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "Employers have tools to keep workers on message.\u{201d} 26 Next sentence."
                .to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: definition.text.clone(),
            label: None,
        },
    ];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&previous, LiquidBlockRole::Paragraph),
                line_ref(&marker, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some(definition.id.clone()),
                page_index: definition.page_index,
                line_index: definition.line_index,
                text: definition.text.clone(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![26],
            }],
        },
    ];

    assert_eq!(
        apply_in_block_standalone_callout_recovery(&mut blocks, &sources, &decoded),
        1
    );
    assert!(blocks[0].text.contains(&format!(
        "message.\u{201d}{CALLOUT_START}26{CALLOUT_END} Next"
    )));
}

#[test]
fn cross_block_superscript_rejoins_prior_paragraph() {
    let mut previous = lm2_test_source_line(
        "p41:l22",
        22,
        "VISIT WWW.TAMKO.COM.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 41;
    previous.page_width = 612.0;
    previous.page_height = 792.0;
    previous.left = 152.52;
    previous.right = 271.71;
    previous.bottom = 437.90;
    previous.top = 446.18;
    previous.font_height = 9.48;
    let mut marker = lm2_test_source_line(
        "p41:l23",
        23,
        "306",
        0.6,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    marker.page_index = 41;
    marker.page_width = 612.0;
    marker.page_height = 792.0;
    marker.left = 271.74;
    marker.right = 283.63;
    marker.bottom = 441.46;
    marker.top = 447.13;
    marker.font_height = 6.48;
    marker.in_footnote_zone = true;
    let mut definition = lm2_test_source_line("p41:l46", 46, "306 Id. at 589.", 0.7, false, None);
    definition.page_index = 41;
    definition.in_footnote_zone = true;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::Marginalia),
        (definition, Lm2Action::Marginalia),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: marker.text.clone(),
            label: None,
        },
    ];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&marker, LiquidBlockRole::Marginalia)],
        },
    ];

    assert_eq!(
        apply_in_block_standalone_callout_recovery(&mut blocks, &sources, &decoded),
        1
    );
    assert!(
        blocks[0]
            .text
            .ends_with(&format!("{CALLOUT_START}306{CALLOUT_END}"))
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Noise);
}

#[test]
fn body_table_cell_with_real_callout_is_not_stranded_as_marginalia() {
    let mut cell = lm2_test_source_line(
        "p71:l11",
        11,
        "Unenforceable495 /",
        0.9,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    cell.page_index = 71;
    cell.page_height = 792.0;
    cell.bottom = 593.56;
    cell.top = 602.89;
    cell.in_footnote_zone = true;
    let mut note = lm2_test_source_line(
        "p71:l44",
        44,
        "495 See Estate of Cawiezell.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 71;
    note.page_height = 792.0;
    note.bottom = 224.63;
    note.top = 235.43;
    note.in_footnote_zone = true;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("Unenforceable{CALLOUT_START}495{CALLOUT_END} /"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut cell_ref = line_ref(&cell, LiquidBlockRole::Marginalia);
    cell_ref.note_markers.clear();
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![495];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![cell_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
    ];
    let decoded = vec![(cell, Lm2Action::Marginalia), (note, Lm2Action::Marginalia)];

    assert_eq!(
        apply_body_callout_marginalia_rescue(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
}

#[test]
fn body_table_cell_with_nonleading_callout_does_not_require_definition_match() {
    let mut cell = lm2_test_source_line(
        "p71:l11",
        11,
        "Unenforceable495 /",
        0.9,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    cell.page_index = 71;
    cell.page_height = 792.0;
    cell.bottom = 593.56;
    cell.top = 602.89;
    cell.in_footnote_zone = true;
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Marginalia,
        text: format!("Unenforceable{CALLOUT_START}495{CALLOUT_END} /"),
        label: None,
    }];
    let mut cell_ref = line_ref(&cell, LiquidBlockRole::Marginalia);
    cell_ref.note_markers.clear();
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![cell_ref],
    }];
    let decoded = vec![(cell, Lm2Action::Marginalia)];

    assert_eq!(
        apply_body_callout_marginalia_rescue(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn learned_footnote_zone_requires_low_page_geometry_for_body_veto() {
    let mut body_callout = lm2_test_source_line("p41:l23", 23, "306", 0.6, false, None);
    body_callout.page_height = 792.0;
    body_callout.top = 447.13;
    body_callout.in_footnote_zone = true;
    assert!(!lm2_low_footnote_zone_evidence(&body_callout));

    let mut note_continuation = lm2_test_source_line(
        "p43:l23",
        23,
        "emergent capabilities in GPT-4",
        0.7,
        false,
        None,
    );
    note_continuation.page_height = 792.0;
    note_continuation.top = 358.24;
    note_continuation.page_has_footnote_divider = true;
    note_continuation.below_footnote_divider = true;
    assert!(lm2_low_footnote_zone_evidence(&note_continuation));

    note_continuation.top = 520.0;
    assert!(lm2_low_footnote_zone_evidence(&note_continuation));
}

#[test]
fn centered_bottom_numeric_furniture_is_suppressed() {
    let mut footer = lm2_test_source_line(
        "p7:l0",
        0,
        "8",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    footer.page_index = 7;
    footer.page_width = 442.8;
    footer.page_height = 686.88;
    footer.left = 219.1;
    footer.right = 223.7;
    footer.bottom = 41.3;
    footer.top = 52.2;
    footer.centered = true;
    footer.segment_block_furniture_like = true;
    let decoded = vec![(footer.clone(), Lm2Action::Marginalia)];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Marginalia,
        text: "8".to_owned(),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![line_ref(&footer, LiquidBlockRole::Marginalia)],
    }];

    assert_eq!(
        apply_numeric_footer_furniture_suppression(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
}

#[test]
fn out_of_note_range_page_zero_furniture_number_is_suppressed() {
    let mut folio = lm2_test_source_line(
        "p0:l0",
        0,
        "1760",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    folio.page_index = 0;
    folio.page_height = 792.0;
    folio.bottom = 119.18;
    folio.top = 129.83;
    folio.centered = false;
    folio.segment_block_furniture_like = true;
    folio.segment_block_table_like = true;
    folio.in_footnote_zone = true;
    let decoded = vec![(folio.clone(), Lm2Action::Marginalia)];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Marginalia,
        text: "1760".to_owned(),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![line_ref(&folio, LiquidBlockRole::Marginalia)],
    }];

    assert_eq!(
        apply_numeric_footer_furniture_suppression(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
}

#[test]
fn sentineled_law_review_running_header_is_suppressed() {
    let mut header = lm2_test_source_line(
        "p2:l0",
        0,
        &format!(
            "978 Michigan Law Review [Vol. {CALLOUT_START}124{CALLOUT_END}:{CALLOUT_START}977{CALLOUT_END}"
        ),
        0.9,
        false,
        Some(LiquidBlockRole::Noise),
    );
    header.page_index = 2;
    header.doc_repeated_edge_text = true;
    let decoded = vec![(header.clone(), Lm2Action::Keep)];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: header.text.clone(),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![line_ref(&header, LiquidBlockRole::Noise)],
    }];

    assert_eq!(
        apply_numeric_footer_furniture_suppression(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
}

#[test]
fn adjacent_duplicate_callout_sentinels_collapse_to_one() {
    let marker = format!("{CALLOUT_START}249{CALLOUT_END}");
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: format!("pensions,{marker}{marker}"),
        label: None,
    }];

    assert_eq!(apply_adjacent_duplicate_callout_suppression(&mut blocks), 1);
    assert_eq!(blocks[0].text, format!("pensions,{marker}"));
}

#[test]
fn standalone_superscript_fragment_attaches_but_note_head_does_not() {
    let mut previous = lm2_test_source_line(
        "p17:l3",
        3,
        "tinker with Rules 23 and 81.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 17;
    previous.page_width = 612.0;
    previous.left = 137.52;
    previous.right = 272.03;
    previous.bottom = 646.81;
    previous.top = 661.67;
    previous.font_height = 10.98;

    let mut superscript = lm2_test_source_line(
        "p17:l4",
        4,
        "137",
        0.6,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    superscript.page_index = 17;
    superscript.page_width = 612.0;
    superscript.left = 271.98;
    superscript.right = 283.87;
    superscript.bottom = 651.16;
    superscript.top = 656.83;
    superscript.font_height = 6.48;
    assert!(lm2_marker_can_attach_to_previous_line(
        &superscript,
        &previous
    ));

    let mut note_head = superscript.clone();
    note_head.left = 137.52;
    note_head.right = 149.41;
    assert!(!lm2_marker_can_attach_to_previous_line(
        &note_head, &previous
    ));
}

#[test]
fn geometric_footnote_zone_marks_lower_small_font_tail() {
    let mut lines = vec![
        lm2_test_source_line(
            "l0",
            0,
            "This is ordinary body text on the page.",
            1.0,
            false,
            None,
        ),
        lm2_test_source_line(
            "l1",
            1,
            "More ordinary body text above the notes.",
            1.0,
            false,
            None,
        ),
        lm2_test_source_line(
            "l18",
            18,
            "A final body line before the notes.",
            1.0,
            false,
            None,
        ),
        lm2_test_source_line("l22", 22, "1. See 123 U.S. 456 (1999).", 0.82, false, None),
        lm2_test_source_line(
            "l23",
            23,
            "Additional citation text continuing the footnote.",
            0.82,
            false,
            None,
        ),
        lm2_test_source_line("l24", 24, "https://example.com/archive", 0.82, false, None),
    ];
    for line in &mut lines {
        line.font_ratio_page_ref = line.font_ratio_page;
    }

    enrich_lm2_geometric_footnote_zone_features(&mut lines);

    assert!(!lines[0].in_footnote_zone);
    assert!(!lines[1].in_footnote_zone);
    assert!(!lines[2].in_footnote_zone);
    assert!(lines[3..].iter().all(|line| line.in_footnote_zone));
}

#[test]
fn geometric_zone_overlay_keeps_heading_like_line() {
    let mut decoded = vec![
        (
            lm2_test_source_line(
                "heading",
                22,
                "APPENDIX",
                0.82,
                true,
                Some(LiquidBlockRole::Heading),
            ),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("note", 23, "1. See 123 U.S. 456 (1999).", 0.82, false, None),
            Lm2Action::Keep,
        ),
    ];
    decoded[0].0.in_footnote_zone = true;
    decoded[0].0.bold = true;
    decoded[0].0.font_ratio_page_ref = 0.82;
    decoded[1].0.in_footnote_zone = true;
    decoded[1].0.font_ratio_page_ref = 0.82;

    apply_d1_runtime_geometric_zone_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
}

fn lm2_zero_runtime(
    marker_decoder_prior: bool,
    small_font_decoder_prior: bool,
    small_font_sequence_prior: bool,
) -> Lm2Runtime {
    Lm2Runtime {
        model_label: "test-zero".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: None,
        pp_footnote_region_membership: false,
        marker_decoder_prior,
        small_font_decoder_prior,
        small_font_sequence_prior,
        anchored_marginalia_flow_guard: false,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test-zero".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: None,
        }),
    }
}

#[test]
fn progressive_preview_limits_pages_and_source_lines() {
    let request = LiquidMode2Request {
        document_epoch: 7,
        path: PathBuf::from("preview.pdf"),
        title: "Preview".to_owned(),
        pages: (0..8).map(|page| format!("page {page}")).collect(),
        deep_source_lines: (0..8)
            .map(|page| {
                let mut line = lm2_test_source_line(
                    &format!("p{page}:l0"),
                    0,
                    &format!("page {page} body"),
                    1.0,
                    false,
                    None,
                );
                line.page_index = page;
                line
            })
            .collect(),
        use_pymupdf_blocks: false,
        use_pp_footnote_regions: false,
        external_emissions_path: None,
        runtime_choice: Lm2RuntimeChoice::CatBoost,
        preview_only: false,
        skip_progressive_preview: false,
    };

    let (preview, page_count) = lm2_progressive_preview_request(&request).unwrap();
    assert_eq!(page_count, LM2_PROGRESSIVE_PREVIEW_PAGES);
    assert_eq!(preview.runtime_choice, request.runtime_choice);
    assert_eq!(preview.pages.len(), LM2_PROGRESSIVE_PREVIEW_PAGES);
    assert_eq!(
        preview.deep_source_lines.len(),
        LM2_PROGRESSIVE_PREVIEW_PAGES
    );
    assert!(
        preview
            .deep_source_lines
            .iter()
            .all(|line| line.page_index < LM2_PROGRESSIVE_PREVIEW_PAGES)
    );
}

#[test]
fn explicit_runtime_choices_override_the_process_default() {
    assert!(!Lm2RuntimeChoice::CatBoost.fasttab_requested());
    assert!(Lm2RuntimeChoice::FastTab.fasttab_requested());
}

fn mark_pp_footnote(line: &mut DeepLiquidSourceLine) {
    line.pp_prior_role = Some("footnote".to_owned());
    line.pp_prior_label = Some("footnote".to_owned());
    line.pp_prior_score = Some(0.92);
}

#[test]
fn legal_cue_does_not_match_substrings() {
    assert!(!has_legal_note_cue("said."));
    assert!(!has_legal_note_cue("paid."));
    assert!(!has_legal_note_cue("tennessee law"));
    assert!(has_legal_note_cue("see also"));
    assert!(has_legal_note_cue("id."));
    assert!(has_legal_note_cue("410 u.s."));
}

#[test]
fn note_start_requires_uppercase_after_marker() {
    assert!(looks_like_note_start("13 Bankruptcy protection"));
    assert!(!looks_like_note_start("13 bankruptcy protection"));
    assert!(!looks_like_note_start("2018 law review"));
}

#[test]
fn marginalia_note_block_start_accepts_punctuated_markers() {
    assert!(looks_like_marginalia_note_block_start(
        "13. Bankruptcy protection"
    ));
    assert!(looks_like_marginalia_note_block_start(
        "13) Bankruptcy protection"
    ));
    assert!(looks_like_marginalia_note_block_start(
        "13] Bankruptcy protection"
    ));
    assert!(!looks_like_marginalia_note_block_start(
        "13. bankruptcy protection"
    ));
    assert!(!looks_like_marginalia_note_block_start(
        "2024. The statute continues"
    ));
}

#[test]
fn longest_ascending_run_rejects_impostor_note_heads() {
    // Pseudocode "1." and "2." inside note 24.
    assert_eq!(
        longest_ascending_run(&[22, 23, 24, 1, 2, 25]),
        HashSet::from([0, 1, 2, 5])
    );
    // Citation volume 106 sitting inside note 11.
    assert_eq!(
        longest_ascending_run(&[10, 11, 106, 12, 13]),
        HashSet::from([0, 1, 3, 4])
    );
    assert_eq!(longest_ascending_run(&[1, 2, 3, 4]).len(), 4);
}

#[test]
fn authoritative_document_markers_reject_citation_shaped_false_head() {
    let mut decoded = Vec::new();
    // Deliberately store the lines out of reading order; source coordinates
    // are authoritative for the document marker sequence.
    for marker in [1, 3, 2, 4] {
        let mut line = lm2_test_source_line(
            &format!("p14:l{marker}"),
            marker,
            &marker.to_string(),
            0.8,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 14;
        line.doc_note_marker = marker as u16;
        decoded.push((line, Lm2Action::Marginalia));
        if marker == 2 {
            let mut false_head = lm2_test_source_line(
                "p14:l20",
                20,
                "55 Fed. Reg. 40791, 40791 (1990).",
                0.8,
                false,
                Some(LiquidBlockRole::Marginalia),
            );
            false_head.page_index = 14;
            false_head.doc_footnote_state = true;
            decoded.push((false_head, Lm2Action::Marginalia));
        }
    }

    let starts = note_start_line_ids_for_scope(&decoded, &[]);

    assert_eq!(starts.len(), 4);
    assert!(!starts.contains("p14:l20"));
    for marker in 1..=4 {
        assert!(starts.contains(&format!("p14:l{marker}")));
    }
}

#[test]
fn note_start_sequence_uses_source_order_not_decoder_storage_order() {
    let mut decoded = Vec::new();
    // The reporter-page impostor is last in decoder storage but sits
    // between 40 and 41 in physical reading order. Storage-order LIS would
    // incorrectly keep it after the real 39-42 run.
    for (line_index, marker) in [
        (0, 39),
        (1, 40),
        (3, 41),
        (4, 42),
        (5, 43),
        (6, 44),
        (7, 45),
        (2, 875),
    ] {
        let id = format!("p8:l{line_index}:n{marker}");
        let mut line = lm2_test_source_line(
            &id,
            line_index,
            &format!("{marker}. Citation text"),
            0.8,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 8;
        line.in_footnote_zone = true;
        decoded.push((line, Lm2Action::Marginalia));
    }

    let starts = note_start_line_ids_for_scope(&decoded, &[]);

    assert_eq!(starts.len(), 7);
    assert!(!starts.contains("p8:l2:n875"));
    for marker in 39..=45 {
        assert!(starts.iter().any(|id| id.ends_with(&format!("n{marker}"))));
    }
}

#[test]
fn article_coordinate_lookup_handles_midpage_boundaries() {
    let spans = vec![
        ArticleSpan {
            article_index: 0,
            start_page_index: 0,
            start_line_index: 0,
            end_page_index: 5,
            end_line_index: 11,
            confidence: 1.0,
            title_hint: None,
            evidence: Vec::new(),
        },
        ArticleSpan {
            article_index: 1,
            start_page_index: 5,
            start_line_index: 11,
            end_page_index: 9,
            end_line_index: 0,
            confidence: 1.0,
            title_hint: None,
            evidence: Vec::new(),
        },
    ];
    assert_eq!(article_index_at(&spans, 5, 10), Some(0));
    assert_eq!(article_index_at(&spans, 5, 11), Some(1));
    assert_eq!(article_index_at(&spans, 8, 99), Some(1));
    assert_eq!(article_index_at(&spans, 9, 0), None);
}

#[test]
fn note_sequence_restarts_independently_in_each_article() {
    let mut decoded = Vec::new();
    for article in 0..2 {
        for marker in 1..=8 {
            let id = format!("a{article}:n{marker}");
            let mut line = lm2_test_source_line(
                &id,
                marker,
                &format!("{marker} Note text"),
                0.8,
                false,
                None,
            );
            line.page_index = article;
            decoded.push((line, Lm2Action::Marginalia));
        }
    }
    let spans = vec![
        ArticleSpan {
            article_index: 0,
            start_page_index: 0,
            start_line_index: 0,
            end_page_index: 1,
            end_line_index: 0,
            confidence: 1.0,
            title_hint: None,
            evidence: Vec::new(),
        },
        ArticleSpan {
            article_index: 1,
            start_page_index: 1,
            start_line_index: 0,
            end_page_index: 2,
            end_line_index: 0,
            confidence: 1.0,
            title_hint: None,
            evidence: Vec::new(),
        },
    ];

    let starts = note_start_line_ids(&decoded, &spans);
    assert_eq!(starts.len(), 16);
    assert!(starts.contains("a0:n1"));
    assert!(starts.contains("a1:n1"));
}

#[test]
fn high_confidence_article_spans_revoke_a_global_note_start() {
    let mut decoded = Vec::new();
    for (page, markers) in [
        (0, vec![1, 1, 1, 1, 1, 1, 1, 1]),
        (1, vec![1, 1, 1, 1, 1, 2, 3, 4]),
    ] {
        for (line_index, marker) in markers.into_iter().enumerate() {
            let id = format!("p{page}:l{line_index}:n{marker}");
            let mut line = lm2_test_source_line(
                &id,
                line_index,
                &format!("{marker} Note text"),
                0.8,
                false,
                None,
            );
            line.page_index = page;
            decoded.push((line, Lm2Action::Marginalia));
        }
    }
    let spans = vec![
        ArticleSpan {
            article_index: 0,
            start_page_index: 0,
            start_line_index: 0,
            end_page_index: 1,
            end_line_index: 0,
            confidence: crate::review_reading::REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN,
            title_hint: None,
            evidence: Vec::new(),
        },
        ArticleSpan {
            article_index: 1,
            start_page_index: 1,
            start_line_index: 0,
            end_page_index: 2,
            end_line_index: 0,
            confidence: crate::review_reading::REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN,
            title_hint: None,
            evidence: Vec::new(),
        },
    ];

    let global = note_start_line_ids_for_scope(&decoded, &[]);
    let scoped_only = note_start_line_ids_for_scope(&decoded, &spans);
    let revoked = note_start_line_ids(&decoded, &spans);
    let one_span = vec![ArticleSpan {
        article_index: 0,
        start_page_index: 0,
        start_line_index: 0,
        end_page_index: 2,
        end_line_index: 0,
        confidence: crate::review_reading::REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN,
        title_hint: None,
        evidence: Vec::new(),
    }];
    assert!(global.contains("p1:l1:n1"));
    assert!(!scoped_only.contains("p1:l1:n1"));
    assert!(!revoked.contains("p1:l1:n1"));
    assert_eq!(global, note_start_line_ids(&decoded, &one_span));
}

#[test]
fn omitted_keep_lines_are_rescued_as_body_paragraphs() {
    let keep = lm2_test_source_line(
        "keep-lost",
        0,
        "we are now to consider the extent to which the company's agent may",
        1.0,
        false,
        None,
    );
    let noise = lm2_test_source_line("noise", 1, "1", 0.6, false, None);
    let decoded = vec![(keep, Lm2Action::Keep), (noise, Lm2Action::HideNoise)];
    let mut blocks = Vec::new();
    let mut sources = Vec::new();
    rescue_omitted_keep_source_lines(&mut blocks, &mut sources, &decoded);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.contains("company's agent"));
    assert_eq!(sources[0].lines[0].id.as_deref(), Some("keep-lost"));
}

#[test]
fn inference_errors_and_invalid_scores_cannot_become_noise_predictions() {
    let line = lm2_test_source_line("p0:l0", 0, "Keep the source prose.", 1.0, false, None);
    let lines = [line];
    let error =
        validate_emissions(&lines, [Err("injected native failure".to_owned())]).unwrap_err();
    assert!(error.contains("page 1, line 1"));
    assert!(error.contains("injected native failure"));
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(validate_emissions(&lines, [Ok([invalid, 0.0, 0.0])]).is_err());
    }
    assert!(validate_emissions(&lines, []).is_err());
    assert!(validate_emissions(&lines, [Ok([0.0; 3]), Ok([0.0; 3])]).is_err());
    assert_eq!(
        validate_emissions(&lines, [Ok([1.0, 2.0, 3.0])]).unwrap(),
        [[1.0, 2.0, 3.0]]
    );
}

#[test]
fn document_generated_layout_priors_do_not_escape_the_request() {
    let mut runtime = lm2_zero_runtime(false, false, false);
    let state = Lm2RequestState::capture(&runtime);
    runtime.pp_priors = Some(Lm2PpPriorIndex {
        source: PathBuf::from("first-document-priors.jsonl"),
        rows: HashMap::new(),
    });
    runtime.pp_footnote_region_membership = true;
    state.restore(&mut runtime);
    assert!(
        runtime.pp_priors.is_none(),
        "the next document must generate its own priors"
    );
    assert!(!runtime.pp_footnote_region_membership);

    runtime.pp_priors = Some(Lm2PpPriorIndex {
        source: PathBuf::from("explicit-shared-priors.jsonl"),
        rows: HashMap::new(),
    });
    runtime.pp_footnote_region_membership = true;
    let state = Lm2RequestState::capture(&runtime);
    runtime.pp_footnote_region_membership = false;
    state.restore(&mut runtime);
    assert_eq!(
        runtime.pp_priors.as_ref().unwrap().source,
        PathBuf::from("explicit-shared-priors.jsonl")
    );
    assert!(runtime.pp_footnote_region_membership);
}

#[test]
fn native_runtime_is_reused_across_two_prepares() {
    let (first, second) = lm2_runtime_reuse_across_two_prepares(Lm2RuntimeChoice::CatBoost);
    assert!(first <= 1);
    assert_eq!(second, 0);
}

#[test]
fn build_lm2_blocks_splits_punctuated_marginalia_note_starts() {
    let decoded = vec![
        (
            lm2_test_source_line(
                "p0:l0",
                0,
                "12 First note begins with cited authority.",
                0.82,
                false,
                None,
            ),
            Lm2Action::Marginalia,
        ),
        (
            lm2_test_source_line(
                "p0:l1",
                1,
                "continues across the extracted PDF line.",
                0.82,
                false,
                None,
            ),
            Lm2Action::Marginalia,
        ),
        (
            lm2_test_source_line(
                "p0:l2",
                2,
                "13. Second note starts with punctuation.",
                0.82,
                false,
                None,
            ),
            Lm2Action::Marginalia,
        ),
    ];

    let (_, blocks, sources) = build_lm2_blocks("", &decoded);
    let marginalia = blocks
        .iter()
        .filter(|block| block.role == LiquidBlockRole::Marginalia)
        .collect::<Vec<_>>();

    assert_eq!(marginalia.len(), 2);
    assert_eq!(
        marginalia[0].text,
        "12 First note begins with cited authority. continues across the extracted PDF line."
    );
    assert_eq!(
        marginalia[1].text,
        "13. Second note starts with punctuation."
    );
    let marginalia_sources = sources
        .iter()
        .filter(|source| {
            blocks
                .get(source.block_index)
                .is_some_and(|block| block.role == LiquidBlockRole::Marginalia)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        marginalia_sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l0", "p0:l1"]
    );
    assert_eq!(
        marginalia_sources[1]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l2"]
    );
}

#[test]
fn build_lm2_blocks_keeps_year_like_marginalia_continuations_together() {
    let decoded = vec![
        (
            lm2_test_source_line(
                "p0:l0",
                0,
                "12 First note begins with cited authority.",
                0.82,
                false,
                None,
            ),
            Lm2Action::Marginalia,
        ),
        (
            lm2_test_source_line(
                "p0:l1",
                1,
                "2024. The statute continued to govern the dispute.",
                0.82,
                false,
                None,
            ),
            Lm2Action::Marginalia,
        ),
    ];

    let (_, blocks, _) = build_lm2_blocks("", &decoded);
    let marginalia = blocks
        .iter()
        .filter(|block| block.role == LiquidBlockRole::Marginalia)
        .collect::<Vec<_>>();

    assert_eq!(marginalia.len(), 1);
    assert_eq!(
        marginalia[0].text,
        "12 First note begins with cited authority. 2024. The statute continued to govern the dispute."
    );
}

#[test]
fn build_lm2_blocks_joins_adjacent_same_row_fragments() {
    let mut first = lm2_test_source_line("p0:l20", 20, "lions of dollars.1", 1.0, false, None);
    first.page_width = 612.0;
    first.page_height = 792.0;
    first.left = 138.24;
    first.right = 217.21;
    first.bottom = 387.02;
    first.top = 397.78;

    let mut second = lm2_test_source_line(
        "p0:l21",
        21,
        "Robers himself received only $500 per loan for his",
        1.0,
        false,
        None,
    );
    second.page_width = 612.0;
    second.page_height = 792.0;
    second.left = 217.20;
    second.right = 476.44;
    second.bottom = 387.01;
    second.top = 397.67;

    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
    let (_, blocks, sources) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        "lions of dollars.1 Robers himself received only $500 per loan for his"
    );
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l20", "p0:l21"]
    );
}

#[test]
fn build_lm2_blocks_keeps_wrap_after_right_side_fragment() {
    let mut first = lm2_test_source_line(
        "p0:l21",
        21,
        "Robers himself received only $500 per loan for his",
        1.0,
        false,
        None,
    );
    first.page_width = 612.0;
    first.page_height = 792.0;
    first.left = 217.20;
    first.right = 476.44;
    first.bottom = 387.01;
    first.top = 397.67;

    let mut second = lm2_test_source_line(
        "p0:l22",
        22,
        "participation in two closings; no payments were ever made, and the",
        1.0,
        false,
        None,
    );
    second.page_width = 612.0;
    second.page_height = 792.0;
    second.left = 138.24;
    second.right = 476.50;
    second.bottom = 375.01;
    second.top = 385.04;

    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
    let (_, blocks, _) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        "Robers himself received only $500 per loan for his participation in two closings; no payments were ever made, and the"
    );
}

#[test]
fn build_lm2_blocks_preserves_centered_all_caps_heading_hint() {
    let mut heading = lm2_test_source_line(
        "p0:l17",
        17,
        "INTRODUCTION",
        0.83,
        true,
        Some(LiquidBlockRole::Heading),
    );
    heading.page_width = 612.0;
    heading.page_height = 792.0;

    let body = lm2_test_source_line(
        "p0:l18",
        18,
        "In 2004 and 2005, Benjamin Robers was a straw buyer",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let decoded = vec![(heading, Lm2Action::Keep), (body, Lm2Action::Keep)];
    let (_, blocks, _) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Heading);
    assert_eq!(blocks[0].text, "INTRODUCTION");
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
}

#[test]
fn build_lm2_blocks_joins_pdf_soft_hyphen_wraps() {
    let first = lm2_test_source_line("p0:l0", 0, "federal prop\u{0002}", 1.0, false, None);
    let second = lm2_test_source_line("p0:l1", 1, "erty interests", 1.0, false, None);
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
    let (_, blocks, _) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "federal property interests");
}

#[test]
fn action_neutral_blocksplit_splits_indented_paragraph_after_sentence_end() {
    let mut first =
        lm2_test_source_line("p0:l0", 0, "This paragraph ends cleanly.", 1.0, false, None);
    first.left = 72.0;
    first.page_width = 612.0;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "The next paragraph starts with an indent.",
        1.0,
        false,
        None,
    );
    second.left = 84.0;
    second.page_width = 612.0;
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

    // Assembly now splits on the first-line indent itself, so the block
    // arrives already separated and the post-pass leaves it alone. The
    // post-pass still matters for the tiers that run it, hence both
    // assertions.
    let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
    assert_eq!(blocks.len(), 2);

    apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(
        sources
            .iter()
            .flat_map(|source| source.lines.iter())
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l0", "p0:l1"]
    );
}

#[test]
fn action_neutral_blocksplit_does_not_split_indented_continuation() {
    let mut first = lm2_test_source_line("p0:l0", 0, "This paragraph continues", 1.0, false, None);
    first.left = 72.0;
    first.page_width = 612.0;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "with an indented wrapped line.",
        1.0,
        false,
        None,
    );
    second.left = 84.0;
    second.page_width = 612.0;
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

    let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
    apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l0", "p0:l1"]
    );
}

#[test]
fn action_neutral_blocksplit_splits_indented_abstract_paragraph() {
    let mut first = lm2_test_source_line(
        "p0:l0",
        0,
        "The opening abstract paragraph ends here.",
        1.0,
        false,
        Some(LiquidBlockRole::Abstract),
    );
    first.left = 58.0;
    first.page_width = 468.0;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "This Article begins a second abstract paragraph.",
        1.0,
        false,
        Some(LiquidBlockRole::Abstract),
    );
    second.left = 74.5;
    second.page_width = 468.0;
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

    let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
    apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

    assert_eq!(blocks.len(), 2);
    assert!(
        blocks
            .iter()
            .all(|block| block.role == LiquidBlockRole::Abstract)
    );
}

#[test]
fn action_neutral_blocksplit_recovers_indent_hidden_by_tall_source_row() {
    let mut first = lm2_test_source_line(
        "p0:l0",
        0,
        "The previous paragraph ends cleanly.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    first.page_width = 612.0;
    first.left = 73.4;
    first.right = 178.0;
    first.bottom = 468.1;
    first.top = 482.5;
    first.font_height = 9.4;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "Voluntarism starts a new paragraph and then soft\u{0002}wraps.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    second.page_width = 612.0;
    second.left = 73.4;
    second.right = 397.3;
    second.bottom = 444.1;
    second.top = 470.5;
    second.font_height = 9.4;
    let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

    let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
    apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "The previous paragraph ends cleanly.");
    assert_eq!(
        blocks[1].text,
        "Voluntarism starts a new paragraph and then soft-wraps."
    );
}

#[test]
fn final_source_backed_split_preserves_recovered_callout_sentinels() {
    let sentinel = format!("{CALLOUT_START}5{CALLOUT_END}");
    let first_text = format!("The previous paragraph ends cleanly.{sentinel}");
    let mut first = lm2_test_source_line(
        "p0:l0",
        0,
        &first_text,
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    first.page_width = 612.0;
    first.left = 73.4;
    first.right = 178.0;
    first.bottom = 468.1;
    first.top = 482.5;
    first.font_height = 9.4;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "Voluntarism starts a new paragraph and soft\u{0002}wraps.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    second.page_width = 612.0;
    second.left = 73.4;
    second.right = 397.3;
    second.bottom = 444.1;
    second.top = 470.5;
    second.font_height = 9.4;
    let decoded = vec![
        (first.clone(), Lm2Action::Keep),
        (second.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: format!(
            "The previous paragraph ends cleanly.{sentinel} Voluntarism starts a new paragraph and soft-wraps."
        ),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&first, LiquidBlockRole::Paragraph),
            line_ref(&second, LiquidBlockRole::Paragraph),
        ],
    }];

    let repaired = apply_final_source_backed_paragraph_splits(&mut blocks, &mut sources, &decoded);

    assert_eq!(repaired, 1);
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.ends_with(&sentinel));
    assert!(blocks[1].text.starts_with("Voluntarism"));
    assert_eq!(sources[0].block_index, 0);
    assert_eq!(sources[1].block_index, 1);
}

#[test]
fn final_source_backed_split_never_slices_inside_callout_sentinel() {
    let mut previous = lm2_test_source_line(
        "p0:l0",
        0,
        "Prior body text.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_width = 612.0;
    previous.left = 73.4;
    previous.right = 178.0;
    let mut marker = lm2_test_source_line(
        "p0:l1",
        1,
        "279",
        0.7,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    marker.page_width = 612.0;
    marker.left = 220.0;
    marker.right = 235.0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::Keep),
    ];
    let sentinel = format!("{CALLOUT_START}279{CALLOUT_END}");
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: format!("Prior body text.{sentinel} Regulators continue the paragraph."),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&previous, LiquidBlockRole::Paragraph),
            line_ref(&marker, LiquidBlockRole::Paragraph),
        ],
    }];

    let repaired = apply_final_source_backed_paragraph_splits(&mut blocks, &mut sources, &decoded);

    assert_eq!(repaired, 0);
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.contains(&sentinel));
}

#[test]
fn deferred_marginalia_bridge_restores_inline_text_before_callout() {
    let before = lm2_test_source_line(
        "p18:l16",
        16,
        "The fundamental power to issue habeas writs is",
        1.0,
        false,
        None,
    );
    let bridge = lm2_test_source_line(
        "p18:l17",
        17,
        "codified at 28 U.S.C. § 2241.",
        1.0,
        false,
        None,
    );
    let marker = format!("{CALLOUT_START}142{CALLOUT_END}");
    let after = lm2_test_source_line(
        "p18:l18",
        18,
        &format!("{marker} Congress has also enacted more specific statutes."),
        1.0,
        false,
        None,
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: bridge.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&bridge, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        format!(
            "The fundamental power to issue habeas writs is codified at 28 U.S.C. § 2241.{marker}"
        )
    );
    assert_eq!(
        blocks[1].text,
        "Congress has also enacted more specific statutes."
    );
}

#[test]
fn deferred_heading_bridge_restores_wrapped_case_citation() {
    let before = lm2_test_source_line(
        "p45:l20",
        20,
        "The Court explained the rule in",
        1.0,
        false,
        None,
    );
    let bridge = lm2_test_source_line(
        "p45:l21",
        21,
        "General Telephone Co. v. Falcon,",
        1.0,
        false,
        None,
    );
    let marker = format!("{CALLOUT_START}365{CALLOUT_END}");
    let after = lm2_test_source_line(
        "p45:l22",
        22,
        &format!("{marker} the Supreme Court explained the requirement."),
        1.0,
        false,
        None,
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: bridge.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&bridge, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        format!("The Court explained the rule in General Telephone Co. v. Falcon,{marker}")
    );
    assert_eq!(
        blocks[1].text,
        "the Supreme Court explained the requirement."
    );
}

#[test]
fn deferred_marginalia_reflow_moves_note_after_open_paragraph() {
    let before = lm2_test_source_line(
        "p0:l0",
        0,
        "This sentence is interrupted by",
        1.0,
        false,
        None,
    );
    let note = lm2_test_source_line("p0:l1", 1, "12 A footnote.", 0.75, false, None);
    let after = lm2_test_source_line("p0:l2", 2, "a note in the middle.", 1.0, false, None);
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        1
    );

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(
        blocks[0].text,
        "This sentence is interrupted by a note in the middle."
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l0", "p0:l2"]
    );
    assert_eq!(sources[1].block_index, 1);
    assert_eq!(
        sources[1]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l1"]
    );
}

#[test]
fn deferred_marginalia_reflow_dehyphenates_across_note() {
    let before = lm2_test_source_line("p0:l0", 0, "It asserts property-", 1.0, false, None);
    let note = lm2_test_source_line("p0:l1", 1, "65 A footnote.", 0.75, false, None);
    let after = lm2_test_source_line("p0:l2", 2, "like rights.", 1.0, false, None);
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks[0].text, "It asserts property-like rights.");
}

#[test]
fn deferred_marginalia_reflow_handles_multi_note_cluster() {
    let before = lm2_test_source_line("p0:l0", 0, "They assume, that", 1.0, false, None);
    let note_a = lm2_test_source_line("p0:l1", 1, "31 First note.", 0.75, false, None);
    let note_b = lm2_test_source_line("p0:l2", 2, "32 Second note.", 0.75, false, None);
    let note_c = lm2_test_source_line("p0:l3", 3, "33 Third note.", 0.75, false, None);
    let after = lm2_test_source_line(
        "p0:l4",
        4,
        "is, democratic accountability continues.",
        1.0,
        false,
        None,
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_a.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_b.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_c.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&note_a, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&note_b, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&note_c, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 4,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(
        blocks[0].text,
        "They assume, that is, democratic accountability continues."
    );
    assert_eq!(
        blocks.iter().map(|block| block.role).collect::<Vec<_>>(),
        vec![
            LiquidBlockRole::Paragraph,
            LiquidBlockRole::Marginalia,
            LiquidBlockRole::Marginalia,
            LiquidBlockRole::Marginalia,
        ]
    );
}

#[test]
fn deferred_marginalia_reflow_reaches_fixed_point_for_chained_notes() {
    let first = lm2_test_source_line("p0:l0", 0, "This starts", 1.0, false, None);
    let note_a = lm2_test_source_line("p0:l1", 1, "1 First note.", 0.75, false, None);
    let middle = lm2_test_source_line("p0:l2", 2, "a sentence that keeps", 1.0, false, None);
    let note_b = lm2_test_source_line("p0:l3", 3, "2 Second note.", 0.75, false, None);
    let last = lm2_test_source_line("p0:l4", 4, "going after another note.", 1.0, false, None);
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: first.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_a.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: middle.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_b.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: last.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&first, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&note_a, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&middle, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&note_b, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 4,
            lines: vec![line_ref(&last, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        2
    );
    assert_eq!(
        blocks[0].text,
        "This starts a sentence that keeps going after another note."
    );
    assert_eq!(
        blocks.iter().map(|block| block.role).collect::<Vec<_>>(),
        vec![
            LiquidBlockRole::Paragraph,
            LiquidBlockRole::Marginalia,
            LiquidBlockRole::Marginalia,
        ]
    );
}

#[test]
fn deferred_marginalia_reflow_keeps_note_after_closed_sentence() {
    let before = lm2_test_source_line("p0:l0", 0, "This paragraph is complete.", 1.0, false, None);
    let note = lm2_test_source_line("p0:l1", 1, "12 A footnote.", 0.75, false, None);
    let after = lm2_test_source_line(
        "p0:l2",
        2,
        "the next line begins lowercase but should stay separate.",
        1.0,
        false,
        None,
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: before.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: after.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
        0
    );
    assert_eq!(blocks[0].text, "This paragraph is complete.");
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
}

#[test]
fn action_neutral_blocksplit_splits_numbered_marginalia_start() {
    let mut first = lm2_test_source_line(
        "p0:l0",
        0,
        "12 First note begins with cited authority.",
        0.82,
        false,
        None,
    );
    first.left = 48.0;
    first.page_width = 612.0;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "13. Second note starts with punctuation.",
        0.82,
        false,
        None,
    );
    second.left = 60.0;
    second.page_width = 612.0;
    let decoded = vec![
        (first, Lm2Action::Marginalia),
        (second, Lm2Action::Marginalia),
    ];

    let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
    apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

    assert_eq!(blocks.len(), 2);
    assert!(
        blocks
            .iter()
            .all(|block| block.role == LiquidBlockRole::Marginalia)
    );
}

#[test]
fn toc_overlay_hides_document_local_dotleader_rows() {
    let dotleader = lm2_test_source_line(
        "p0:l0",
        0,
        "I. THEORETICAL BACKGROUND................................. 2260",
        0.88,
        false,
        None,
    );
    let body = lm2_test_source_line(
        "p0:l1",
        1,
        "Ordinary article prose continues here.",
        1.0,
        false,
        None,
    );
    let mut decoded = vec![(dotleader, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

    apply_document_toc_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[1].1, Lm2Action::Keep);
}

#[test]
fn strict_no_dot_toc_guard_is_bounded_before_real_body() {
    let texts = [
        "article contents",
        "introduction 2958",
        "i. from police federalism to cross-sovereign enforcement 2966",
        "A. The Construction of Police Federalism 2967",
        "1. Authority Attribution and the Reclamation of section 1983 3003",
        "This real article paragraph begins after the contents run.",
    ];
    let mut decoded = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let mut line =
                lm2_test_source_line(&format!("p2:l{index}"), index, text, 1.0, false, None);
            line.page_index = 2;
            (line, Lm2Action::Keep)
        })
        .collect::<Vec<_>>();

    assert_eq!(apply_strict_no_dot_toc_page_guard(&mut decoded), 5);
    assert!(decoded[..5].iter().all(|(line, action)| {
        *action == Lm2Action::HideNoise && line.role_hint == Some(LiquidBlockRole::Contents)
    }));
    assert_eq!(decoded[5].1, Lm2Action::Keep);
    assert_eq!(apply_hidden_numbered_note_head_recovery(&mut decoded), 0);
    assert_eq!(decoded[4].1, Lm2Action::HideNoise);
}

#[test]
fn article_span_filter_rejects_numbered_figure_boundary() {
    let mut caption = lm2_test_source_line(
        "p10:l1",
        1,
        "FIGURE 2. STRATEGIC MOOTNESS GAP TACTICS",
        1.2,
        true,
        None,
    );
    caption.page_index = 10;
    let decoded = vec![(caption, Lm2Action::Keep)];
    let detected = vec![
        ArticleSpan {
            article_index: 0,
            start_page_index: 0,
            start_line_index: 0,
            end_page_index: 10,
            end_line_index: 0,
            confidence: 1.0,
            title_hint: Some("Opening article".to_owned()),
            evidence: Vec::new(),
        },
        ArticleSpan {
            article_index: 1,
            start_page_index: 10,
            start_line_index: 0,
            end_page_index: 20,
            end_line_index: 0,
            confidence: 0.5,
            title_hint: Some("FIGURE 2. STRATEGIC MOOTNESS GAP TACTICS".to_owned()),
            evidence: Vec::new(),
        },
    ];

    let filtered = filter_lm2_false_article_spans(detected, &decoded, 20);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].end_page_index, 20);
}

#[test]
fn isolated_body_noise_tail_recovers_same_font_sentence_end() {
    let previous = lm2_test_source_line(
        "p10:l29",
        29,
        "Entries with a single date indicate that the commission remained inquorate as of",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut tail = lm2_test_source_line(
        "p10:l30",
        30,
        "October 7, 2025.",
        1.0,
        false,
        Some(LiquidBlockRole::Table),
    );
    tail.segment_block_shape = "table".to_owned();
    tail.segment_block_table_like = true;
    let mut decoded = vec![(previous, Lm2Action::Keep), (tail, Lm2Action::HideNoise)];

    assert_eq!(apply_isolated_body_noise_tail_recovery(&mut decoded), 1);
    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Paragraph));
}

#[test]
fn numbered_table_caption_and_cross_page_cells_survive_real_note_dividers() {
    let mut header = lm2_test_source_line(
        "p11:l0",
        0,
        "Commission Quorums",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    header.page_index = 11;
    header.doc_repeated_text_count = 6;
    let mut folio = lm2_test_source_line(
        "p11:l1",
        1,
        "1136",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    folio.page_index = 11;
    let mut caption = lm2_test_source_line(
        "p11:l3",
        3,
        "Table 1",
        1.0,
        true,
        Some(LiquidBlockRole::Noise),
    );
    caption.page_index = 11;
    let mut subtitle = lm2_test_source_line(
        "p11:l4",
        4,
        "Commissions Without Quorums, January-October 7, 2025",
        1.0,
        true,
        Some(LiquidBlockRole::Noise),
    );
    subtitle.page_index = 11;
    let mut table_header = lm2_test_source_line(
        "p11:l5",
        5,
        "Commission Dates Without Quorum",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    table_header.page_index = 11;
    table_header.page_object_path_stroke_near_line_count = 1;
    let mut row = lm2_test_source_line(
        "p11:l6",
        6,
        "Defense Nuclear Facilities Safety Board",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    row.page_index = 11;
    row.page_object_path_stroke_near_line_count = 1;
    let mut note = lm2_test_source_line(
        "p11:l7",
        7,
        "65. Press Release describing the quorum loss.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 11;
    note.in_footnote_zone = true;

    let mut next_header = lm2_test_source_line(
        "p12:l0",
        0,
        "Commission Quorums",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    next_header.page_index = 12;
    next_header.doc_repeated_text_count = 6;
    let mut next_folio = lm2_test_source_line(
        "p12:l1",
        1,
        "1137",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    next_folio.page_index = 12;
    let mut continued = lm2_test_source_line(
        "p12:l3",
        3,
        "Internal Revenue Service Oversight",
        1.0,
        false,
        Some(LiquidBlockRole::Noise),
    );
    continued.page_index = 12;
    continued.page_object_path_stroke_near_line_count = 1;
    let mut continued_row = lm2_test_source_line(
        "p12:l4",
        4,
        "Board 11/2013 Term Expiration",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    continued_row.page_index = 12;
    continued_row.page_object_path_stroke_near_line_count = 1;
    let mut next_note = lm2_test_source_line(
        "p12:l5",
        5,
        "72. Treasury Inspector General report.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 12;
    next_note.in_footnote_zone = true;

    let mut decoded = vec![
        (header, Lm2Action::HideNoise),
        (folio, Lm2Action::HideNoise),
        (caption, Lm2Action::HideNoise),
        (subtitle, Lm2Action::HideNoise),
        (table_header, Lm2Action::Keep),
        (row, Lm2Action::Keep),
        (note, Lm2Action::Marginalia),
        (next_header, Lm2Action::HideNoise),
        (next_folio, Lm2Action::HideNoise),
        (continued, Lm2Action::HideNoise),
        (continued_row, Lm2Action::Keep),
        (next_note, Lm2Action::Marginalia),
    ];

    apply_numbered_table_figure_band_role_hints(&mut decoded);

    assert_eq!(decoded[2].0.role_hint, Some(LiquidBlockRole::Caption));
    assert_eq!(decoded[3].0.role_hint, Some(LiquidBlockRole::Caption));
    assert!(decoded[4..6].iter().all(|(line, action)| {
        *action == Lm2Action::HideNoise && line.role_hint == Some(LiquidBlockRole::Table)
    }));
    assert!(decoded[9..11].iter().all(|(line, action)| {
        *action == Lm2Action::HideNoise && line.role_hint == Some(LiquidBlockRole::Table)
    }));
    assert_eq!(decoded[6].1, Lm2Action::Marginalia);
    assert_eq!(decoded[11].1, Lm2Action::Marginalia);
}

#[test]
fn numbered_table_band_stops_before_indented_body_return() {
    let mut caption = lm2_test_source_line("p49:l3", 3, "Table 4", 1.0, true, None);
    caption.page_width = 1000.0;
    let mut subtitle = lm2_test_source_line("p49:l4", 4, "Preventive Structures", 1.0, true, None);
    subtitle.page_width = 1000.0;
    let mut row = lm2_test_source_line(
        "p49:l5",
        5,
        "Holdover The ability of members to serve after term expiration",
        1.0,
        false,
        None,
    );
    row.page_width = 1000.0;
    row.left = 250.0;
    row.right = 900.0;
    row.page_object_path_stroke_near_line_count = 1;
    let mut final_row = lm2_test_source_line(
        "p49:l6",
        6,
        "The requirement that vacancies be filled within a fixed period",
        1.0,
        false,
        None,
    );
    final_row.page_width = 1000.0;
    final_row.left = 300.0;
    final_row.right = 800.0;
    final_row.page_object_path_stroke_near_line_count = 1;
    let mut body_first = lm2_test_source_line(
        "p49:l7",
        7,
        "Two preventive structures are especially important in limiting",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body_first.page_width = 1000.0;
    body_first.left = 250.0;
    body_first.right = 930.0;
    let mut body_next = lm2_test_source_line(
        "p49:l8",
        8,
        "commissions' quorum losses and preserving institutional authority",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body_next.page_width = 1000.0;
    body_next.left = 220.0;
    body_next.right = 940.0;
    let mut decoded = vec![
        (caption, Lm2Action::Keep),
        (subtitle, Lm2Action::Keep),
        (row, Lm2Action::Keep),
        (final_row, Lm2Action::Keep),
        (body_first, Lm2Action::Keep),
        (body_next, Lm2Action::Keep),
    ];

    apply_numbered_table_figure_band_role_hints(&mut decoded);

    assert!(decoded[2..4].iter().all(|(line, action)| {
        *action == Lm2Action::HideNoise && line.role_hint == Some(LiquidBlockRole::Table)
    }));
    assert!(decoded[4..].iter().all(|(line, action)| {
        *action == Lm2Action::Keep && line.role_hint == Some(LiquidBlockRole::Paragraph)
    }));
}

#[test]
fn numbered_figure_band_stops_before_body_below_false_divider() {
    let texts = [
        "FIGURE 1. MOOTNESS EXCEPTIONS",
        "Inherent Mootness Strategic Mootness",
        "Example: Pregnancy litigation",
        "This Article identifies and addresses the strategic mootness gap.",
    ];
    let mut decoded = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let mut line =
                lm2_test_source_line(&format!("p4:l{index}"), index, text, 1.0, false, None);
            line.page_index = 4;
            if index == 3 {
                line.page_has_footnote_divider = true;
                line.below_footnote_divider = true;
            }
            (line, Lm2Action::Keep)
        })
        .collect::<Vec<_>>();

    assert_eq!(apply_numbered_table_figure_band_role_hints(&mut decoded), 3);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Caption));
    assert!(decoded[1..3].iter().all(|(line, action)| {
        *action == Lm2Action::HideNoise && line.role_hint == Some(LiquidBlockRole::Table)
    }));
    assert_eq!(decoded[3].1, Lm2Action::Keep);
    assert_eq!(decoded[3].0.role_hint, None);
}

#[test]
fn numbered_table_reference_in_note_zone_is_not_routed_as_caption() {
    let mut reference = lm2_test_source_line(
        "p9:l35",
        35,
        "Table 4",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    reference.in_footnote_zone = true;
    reference.doc_footnote_state = true;
    let mut decoded = vec![(reference, Lm2Action::Marginalia)];

    assert_eq!(apply_numbered_table_figure_band_role_hints(&mut decoded), 0);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
}

#[test]
fn table_router_hides_short_heading_labels_below_embedded_table_caption() {
    let anchor = lm2_test_source_line(
        "p11:l0",
        0,
        "Commission Quorums 1136 Table 1 Commissions Without Quorums",
        0.9,
        false,
        Some(LiquidBlockRole::Noise),
    );
    let label = lm2_test_source_line(
        "p11:l9",
        9,
        "Defense Nuclear Facilities Safety",
        1.0,
        false,
        Some(LiquidBlockRole::Heading),
    );
    let second_label = lm2_test_source_line(
        "p11:l12",
        12,
        "Equal Employment Opportunity Commission",
        1.0,
        false,
        Some(LiquidBlockRole::Heading),
    );
    let decoded = vec![
        (anchor, Lm2Action::HideNoise),
        (label.clone(), Lm2Action::Keep),
        (second_label, Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Heading,
        text: label.text.clone(),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![line_ref(&label, LiquidBlockRole::Heading)],
    }];

    assert!(lm2_numbered_table_figure_page_anchor(&decoded[0].0.text));
    assert_eq!(
        apply_table_figure_block_role_routing(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Table);
}

#[test]
fn table_router_preserves_a_source_backed_roman_outline_heading() {
    let mut first = lm2_test_source_line(
        "p52:l2",
        2,
        "IV. FROM LEGALISTIC NONCOMPLIANCE TO LEGALIZED",
        1.2,
        true,
        Some(LiquidBlockRole::Paragraph),
    );
    first.in_ruled_cell = true;
    let mut second = lm2_test_source_line(
        "p52:l3",
        3,
        "NONCOMPLIANCE?",
        1.2,
        true,
        Some(LiquidBlockRole::Heading),
    );
    second.in_ruled_cell = true;
    let decoded = vec![
        (first.clone(), Lm2Action::Keep),
        (second.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Heading,
        text: format!("{} {}", first.text, second.text),
        label: None,
    }];
    let sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&first, LiquidBlockRole::Paragraph),
            line_ref(&second, LiquidBlockRole::Heading),
        ],
    }];

    assert_eq!(
        apply_table_figure_block_role_routing(&mut blocks, &sources, &decoded),
        0
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Heading);
}

#[test]
fn lowercase_bold_conclusion_splits_from_its_paragraph_body() {
    let mut heading = lm2_test_source_line(
        "p79:l17",
        17,
        "conclusion",
        1.08,
        true,
        Some(LiquidBlockRole::Paragraph),
    );
    heading.bold = true;
    heading.segment_block_shape = "heading".to_owned();
    let body = lm2_test_source_line(
        "p79:l18",
        18,
        "Cross-sovereign policing is a constitutional anomaly.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let decoded = vec![
        (heading.clone(), Lm2Action::Keep),
        (body.clone(), Lm2Action::Keep),
    ];
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let refs = vec![
        line_ref(&heading, LiquidBlockRole::Paragraph),
        line_ref(&body, LiquidBlockRole::Paragraph),
    ];

    let groups = lm2_paragraph_outline_groups(&refs, &line_by_id).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, LiquidBlockRole::Heading);
    assert_eq!(groups[0].1[0].text, "conclusion");
}

#[test]
fn source_note_markers_require_an_accepted_note_start() {
    let mut head = lm2_test_source_line(
        "p4:l16",
        16,
        &format!("{CALLOUT_START}12{CALLOUT_END}. Authority."),
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    head.in_footnote_zone = true;
    head.segment_block_footnote_like = true;
    let body = lm2_test_source_line(
        "p4:l17",
        17,
        &format!("Body prose continues{CALLOUT_START}12{CALLOUT_END} here."),
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut explicit = lm2_test_source_line(
        "p4:l18",
        18,
        "12. Authority.",
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    explicit.in_footnote_zone = true;
    explicit.font_ratio_page_ref = 0.82;
    explicit.font_ratio_page = 0.82;
    explicit.font_ratio_doc = 0.82;

    assert!(source_line_note_markers(&head, LiquidBlockRole::Marginalia).is_empty());
    assert!(source_line_note_markers(&body, LiquidBlockRole::Marginalia).is_empty());
    assert_eq!(
        source_line_note_markers(&explicit, LiquidBlockRole::Marginalia),
        vec![12]
    );
    assert_eq!(
        line_ref_with_note_start(&head, LiquidBlockRole::Marginalia, true).note_markers,
        vec![12]
    );
    assert!(
        line_ref_with_note_start(&body, LiquidBlockRole::Marginalia, false)
            .note_markers
            .is_empty()
    );
}

#[test]
fn dense_page_note_head_can_sit_above_page_midpoint() {
    let mut note = lm2_test_source_line(
        "p71:l16",
        16,
        &format!("{CALLOUT_START}353{CALLOUT_END}. Authority."),
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_height = 720.0;
    note.top = 444.5;
    note.bottom = 433.1;
    note.in_footnote_zone = true;
    note.segment_block_footnote_like = true;
    note.font_ratio_page_ref = 0.82;

    let mut body = note.clone();
    body.id = "p71:l15".to_owned();
    body.line_index = 15;
    body.text = format!("{CALLOUT_START}356{CALLOUT_END} What begins as");
    body.in_footnote_zone = false;
    body.segment_block_footnote_like = false;
    body.font_ratio_page_ref = 0.91;

    assert!(lm2_note_head_zone_evidence(&note));
    assert_eq!(leading_sentineled_note_head_marker(&note), Some(353));
    assert!(!lm2_note_head_zone_evidence(&body));
    assert_eq!(leading_sentineled_note_head_marker(&body), None);

    let mut missed_zone = note.clone();
    missed_zone.id = "p4:l16".to_owned();
    missed_zone.text = format!("{CALLOUT_START}4{CALLOUT_END}. See authority.");
    missed_zone.in_footnote_zone = false;
    missed_zone.segment_block_footnote_like = false;
    missed_zone.segment_block_shape = "body".to_owned();
    missed_zone.font_ratio_page_ref = 0.81;
    assert!(!lm2_note_head_zone_evidence(&missed_zone));
    assert_eq!(leading_sentineled_note_head_marker(&missed_zone), Some(4));
}

#[test]
fn same_row_callout_with_following_prose_uses_definition_evidence() {
    let mut previous = lm2_test_source_line(
        "p30:l36",
        36,
        "on the ground that the local agents acted under color of federal law.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 30;
    previous.page_width = 486.0;
    previous.left = 57.96;
    previous.right = 351.72;
    previous.bottom = 250.24;
    previous.top = 261.20;
    previous.font_height = 9.24;
    let mut current = lm2_test_source_line(
        "p30:l37",
        37,
        "153 However,",
        0.97,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    current.page_index = 30;
    current.page_width = 486.0;
    current.left = 351.84;
    current.right = 407.82;
    current.bottom = 250.24;
    current.top = 261.82;
    current.font_height = 9.02;
    current.in_footnote_zone = true;

    assert!(lm2_same_row_leading_callout_with_prose(
        &previous, &current, 153
    ));

    previous.id = "p13:l13".to_owned();
    previous.page_index = 13;
    previous.line_index = 13;
    previous.text =
        "independent review will infringe on that organizationâ€™s expressive association rights."
            .to_owned();
    previous.left = 73.4;
    previous.right = 397.4;
    previous.bottom = 456.1;
    previous.top = 482.5;
    previous.font_height = 9.36;
    current.id = "p13:l14".to_owned();
    current.page_index = 13;
    current.line_index = 14;
    current.text = "78 A close".to_owned();
    current.left = 358.7;
    current.right = 397.5;
    current.bottom = 456.1;
    current.top = 470.5;
    current.font_height = 7.82;
    current.in_footnote_zone = false;
    assert!(lm2_same_row_leading_callout_with_prose(
        &previous, &current, 78
    ));
}

#[test]
fn mixed_marginalia_moves_only_body_callout_line_back_to_paragraph() {
    let mut previous = lm2_test_source_line(
        "p8:l21",
        21,
        "ory—renders such public coercion reviewable and legitimate.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 8;
    previous.page_width = 486.0;
    previous.left = 57.96;
    previous.right = 325.95;
    previous.bottom = 408.22;
    previous.top = 419.15;
    previous.font_height = 9.97;
    let mut callout = lm2_test_source_line(
        "p8:l22",
        22,
        &format!("{CALLOUT_START}29{CALLOUT_END} If sovereign ide-"),
        0.91,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    callout.page_index = 8;
    callout.page_width = 486.0;
    callout.left = 326.04;
    callout.right = 407.77;
    callout.bottom = 408.22;
    callout.top = 419.54;
    callout.font_height = 9.03;
    let mut continuation = lm2_test_source_line(
        "p8:l23",
        23,
        "Common Law, Cooperative Federalism, and the Enforcement of the Telecom Act.",
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    continuation.page_index = 8;
    continuation.page_height = 720.0;
    continuation.top = 190.0;
    continuation.in_footnote_zone = true;
    continuation.segment_block_footnote_like = true;
    let mut definition = continuation.clone();
    definition.id = "p8:l38".to_owned();
    definition.line_index = 38;
    definition.text = format!("{CALLOUT_START}29{CALLOUT_END}. Definition.");

    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (callout.clone(), Lm2Action::Marginalia),
        (continuation.clone(), Lm2Action::Marginalia),
        (definition.clone(), Lm2Action::Marginalia),
    ];
    let mut definition_ref = line_ref(&definition, LiquidBlockRole::Marginalia);
    definition_ref.note_markers.push(29);
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{} {}", callout.text, continuation.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: definition.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&callout, LiquidBlockRole::Marginalia),
                line_ref(&continuation, LiquidBlockRole::Marginalia),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![definition_ref],
        },
    ];

    assert_eq!(
        apply_mixed_marginalia_leading_body_callout_split(&mut blocks, &mut sources, &decoded,),
        1
    );
    assert!(blocks[0].text.contains(&format!(
        "legitimate.{CALLOUT_START}29{CALLOUT_END} If sovereign ide-"
    )));
    assert_eq!(blocks[1].text, continuation.text);
    assert_eq!(sources[0].lines.len(), 2);
    assert_eq!(sources[1].lines.len(), 1);
}

#[test]
fn abstract_continuation_and_author_label_leave_marginalia() {
    let abstract_line = lm2_test_source_line(
        "p0:l10",
        10,
        "The abstract begins here.",
        0.9,
        false,
        Some(LiquidBlockRole::Abstract),
    );
    let mut continuation = lm2_test_source_line(
        "p1:l2",
        2,
        "committed under color of federal law.",
        0.85,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    continuation.page_index = 1;
    let mut author = continuation.clone();
    author.id = "p1:l7".to_owned();
    author.line_index = 7;
    author.text = "author. Gary & Sallyn Pajcic Professor.".to_owned();
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Abstract,
            text: abstract_line.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{} {}", continuation.text, author.text),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&abstract_line, LiquidBlockRole::Abstract)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&continuation, LiquidBlockRole::Marginalia),
                line_ref(&author, LiquidBlockRole::Marginalia),
            ],
        },
    ];

    assert_eq!(
        apply_front_matter_abstract_author_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::AuthorInfo);
    assert!(blocks[0].text.ends_with(&continuation.text));
    assert!(blocks[1].text.starts_with("author."));
}

#[test]
fn starred_byline_is_split_from_unlabelled_cross_page_abstract() {
    let mut byline = lm2_test_source_line(
        "p0:l7",
        7,
        "Rachel Bayefsky*",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    byline.page_index = 0;
    let mut opening = lm2_test_source_line(
        "p0:l8",
        8,
        "In recent years, tradition has been influentially invoked in constitutional rights adjudication.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    opening.page_index = 0;
    let mut author_note = lm2_test_source_line(
        "p0:l20",
        20,
        "* Associate Professor of Law. Thanks to the editors.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    author_note.page_index = 0;
    let mut header = lm2_test_source_line(
        "p1:l0",
        0,
        "866 Virginia Law Review [Vol. 112:865",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    header.page_index = 1;
    let mut continuation = lm2_test_source_line(
        "p1:l2",
        2,
        "subordination? Yet the relationship between feminism and traditionalism depends on the form that traditionalism takes.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    continuation.page_index = 1;
    let mut contents = lm2_test_source_line(
        "p1:l24",
        24,
        "I. TRADITION 875 A. DOCTRINE 876 B. MEANING 883 II. AN ACCOUNT 889 CONCLUSION 945",
        0.8,
        false,
        Some(LiquidBlockRole::Noise),
    );
    contents.page_index = 1;
    let mut introduction = lm2_test_source_line(
        "p2:l14",
        14,
        "INTRODUCTION",
        1.1,
        false,
        Some(LiquidBlockRole::Heading),
    );
    introduction.page_index = 2;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Title,
            text: "TRADITION AND FEMINISM IN CONSTITUTIONAL RIGHTS ADJUDICATION".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", byline.text, opening.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: author_note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: header.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: continuation.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: contents.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: introduction.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&byline, LiquidBlockRole::Paragraph),
                line_ref(&opening, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&author_note, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&header, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 4,
            lines: vec![line_ref(&continuation, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 5,
            lines: vec![line_ref(&contents, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 6,
            lines: vec![line_ref(&introduction, LiquidBlockRole::Heading)],
        },
    ];

    assert_eq!(
        apply_front_matter_byline_abstract_split(&mut blocks, &mut sources),
        2
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::AuthorInfo);
    assert_eq!(blocks[1].text, "Rachel Bayefsky*");
    assert_eq!(blocks[2].role, LiquidBlockRole::Abstract);
    assert!(blocks[2].text.starts_with("In recent years"));
    assert_eq!(blocks[5].role, LiquidBlockRole::Abstract);
    assert!(blocks[5].text.starts_with("subordination?"));
    assert_eq!(blocks[6].role, LiquidBlockRole::Noise);
    assert_eq!(blocks[7].role, LiquidBlockRole::Heading);
    assert_eq!(sources[0].block_index, 1);
    assert_eq!(sources[0].lines[0].role, LiquidBlockRole::AuthorInfo);
    assert_eq!(sources[1].block_index, 2);
    assert!(
        sources[1]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Abstract)
    );
}

#[test]
fn toc_overlay_recovers_matching_section_heading() {
    let dotleader = lm2_test_source_line(
        "p0:l0",
        0,
        "I. THEORETICAL BACKGROUND................................. 2260",
        0.88,
        false,
        None,
    );
    let mut heading =
        lm2_test_source_line("p2:l4", 4, "I. Theoretical Background", 1.05, false, None);
    heading.page_index = 2;
    heading.bold = true;
    let mut decoded = vec![
        (dotleader, Lm2Action::HideNoise),
        (heading, Lm2Action::Marginalia),
    ];

    apply_document_toc_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Heading));

    let (_, blocks, _) = build_lm2_blocks("", &decoded);
    assert!(
        blocks
            .iter()
            .any(|block| block.role == LiquidBlockRole::Heading
                && block.text == "I. Theoretical Background")
    );
}

#[test]
fn front_matter_guard_hides_first_page_noise_hint_masthead() {
    let mut line = lm2_test_source_line(
        "p0:l2",
        2,
        "VIRGINIA LAW REVIEW",
        1.8,
        true,
        Some(LiquidBlockRole::Noise),
    );
    line.top = 0.88;
    line.bottom = 0.84;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_front_matter_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn static_front_overlay_does_not_promote_noise_hint_masthead() {
    let mut rows = HashMap::new();
    rows.insert("p0:l2".to_owned(), LiquidBlockRole::Title);
    let mut roles_by_doc_line = HashMap::new();
    roles_by_doc_line.insert("doc.pdf".to_owned(), rows);
    let overlay = Lm2StaticFrontOverlay {
        source_label: "test".to_owned(),
        roles_by_doc_line,
    };
    let mut line = lm2_test_source_line(
        "p0:l2",
        2,
        "VIRGINIA LAW REVIEW",
        1.8,
        true,
        Some(LiquidBlockRole::Noise),
    );
    line.top = 0.88;
    line.bottom = 0.84;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_static_front_overlay(&overlay, Path::new("doc.pdf"), &mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn front_matter_guard_hides_first_page_repository_boilerplate() {
    let mut line = lm2_test_source_line(
        "p0:l14",
        14,
        "History. It has been accepted for inclusion in Fordham Law Review Archive by an authorized editor.",
        0.9,
        true,
        Some(LiquidBlockRole::Noise),
    );
    line.top = 0.14;
    line.bottom = 0.12;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_front_matter_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
}

#[test]
fn front_matter_guard_hides_repeated_top_edge_noise_furniture() {
    let mut line = lm2_test_source_line(
        "p5:l0",
        0,
        "COPYRIGHT © 2026 VIRGINIA LAW REVIEW ASSOCIATION",
        0.50,
        false,
        None,
    );
    line.page_index = 5;
    line.top = 0.953;
    line.bottom = 0.941;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_front_matter_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
}

#[test]
fn front_matter_guard_hides_first_page_masthead_without_role_hint() {
    let mut line = lm2_test_source_line("p0:l2", 2, "VIRGINIA LAW REVIEW", 1.8, true, None);
    line.top = 0.88;
    line.bottom = 0.84;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_front_matter_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
}

#[test]
fn front_matter_guard_demotes_first_page_author_byline() {
    let mut line = lm2_test_source_line(
        "p0:l5",
        5,
        "Aric Short† and Tanya Pierce††",
        1.05,
        true,
        None,
    );
    line.top = 0.72;
    line.bottom = 0.70;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_front_matter_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
}

#[test]
fn page_label_furniture_guard_hides_page_x_of_y_labels() {
    let line = lm2_test_source_line("p14:l0", 0, "Page 14 of 26", 0.84, false, None);
    let mut decoded = vec![(line, Lm2Action::Marginalia)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn page_label_furniture_guard_hides_page_x_labels() {
    let line = lm2_test_source_line("p4:l0", 0, "Page 4", 0.84, false, None);
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn page_label_furniture_guard_does_not_hide_prose_with_page_word() {
    let line = lm2_test_source_line(
        "p9:l12",
        12,
        "Page limits can affect appellate briefing schedules.",
        1.0,
        false,
        None,
    );
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn page_label_furniture_guard_hides_centered_numeric_with_noise_hint() {
    let line = lm2_test_source_line("p1:l18", 18, "5", 0.59, true, Some(LiquidBlockRole::Noise));
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn page_label_furniture_guard_hides_parenthesized_numeric_with_marginalia_hint() {
    let line = lm2_test_source_line(
        "p0:l1",
        1,
        "(1139)",
        0.86,
        true,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn page_label_furniture_guard_keeps_unhinted_centered_numeric_line() {
    let line = lm2_test_source_line("p3:l8", 8, "12", 1.0, true, None);
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn page_label_furniture_guard_keeps_noncentered_numeric_with_noise_hint() {
    let line = lm2_test_source_line(
        "p4:l22",
        22,
        "22",
        0.80,
        false,
        Some(LiquidBlockRole::Noise),
    );
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_page_label_furniture_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn marginalia_preservation_guard_recovers_hidden_url_continuation() {
    let mut line = lm2_test_source_line(
        "p14:l33",
        33,
        "nviction/ [https://perma.cc/U6R4-UBLJ] (last visited Feb. 11, 2026) (\"State post-conviction",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    line.page_index = 14;
    line.bottom = 0.16;
    line.top = 0.18;
    line.page_has_footnote_divider = true;
    let mut decoded = vec![(line, Lm2Action::HideNoise)];

    apply_marginalia_preservation_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
}

#[test]
fn marginalia_preservation_guard_recovers_url_continuation_without_hint() {
    let mut line = lm2_test_source_line(
        "p10:l29",
        29,
        "innocenceproject.org/dna-exonerations-in-the-united-states/ [https://perma.cc/V4GA-ZCJT]",
        0.80,
        false,
        None,
    );
    line.page_index = 10;
    line.bottom = 266.994;
    line.top = 279.11697;
    line.page_height = 792.0;
    let mut decoded = vec![(line, Lm2Action::HideNoise)];

    apply_marginalia_preservation_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
}

#[test]
fn marginalia_preservation_guard_does_not_recover_toc_dotleader() {
    let mut line = lm2_test_source_line(
        "p1:l10",
        10,
        "A. Governance ..................................................... 1613",
        0.88,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    line.page_index = 1;
    line.bottom = 0.18;
    line.top = 0.20;
    line.page_has_footnote_divider = true;
    let mut decoded = vec![(line, Lm2Action::HideNoise)];

    apply_marginalia_preservation_guard(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
}

#[test]
fn append_line_repairs_simple_dehyphenation() {
    let mut text = "applica-".to_owned();
    append_line(&mut text, "tion");
    assert_eq!(text, "application");
}

#[test]
fn append_line_preserves_common_terminal_hyphen_compounds() {
    let mut state_court = "state-".to_owned();
    append_line(&mut state_court, "court adjudication");
    assert_eq!(state_court, "state-court adjudication");

    let mut case_specific = "case-".to_owned();
    append_line(&mut case_specific, "specific inquiry");
    assert_eq!(case_specific, "case-specific inquiry");

    let mut conflict_of_laws = "conflict-".to_owned();
    append_line(&mut conflict_of_laws, "of-laws analysis");
    assert_eq!(conflict_of_laws, "conflict-of-laws analysis");

    let mut conflicting = "conflict-".to_owned();
    append_line(&mut conflicting, "ing decisions");
    assert_eq!(conflicting, "conflicting decisions");

    let mut abortion_rights_group = "an abortion-".to_owned();
    append_line(&mut abortion_rights_group, "rights group would object");
    assert_eq!(
        abortion_rights_group,
        "an abortion-rights group would object"
    );
}

#[test]
fn append_line_does_not_add_space_after_terminal_typographic_dash() {
    let mut em_dash = "rights are secured\u{2014}".to_owned();
    append_line(&mut em_dash, "Congress's response");
    assert_eq!(em_dash, "rights are secured\u{2014}Congress's response");

    let mut en_dash = "the state\u{2013}".to_owned();
    append_line(&mut en_dash, "federal divide");
    assert_eq!(en_dash, "the state\u{2013}federal divide");
}

#[test]
fn document_hyphen_evidence_repairs_internal_pdf_markers() {
    let evidence_joined = lm2_test_source_line(
        "p0:l0",
        0,
        "Authority and statement are intact elsewhere.",
        1.0,
        false,
        None,
    );
    let evidence_hyphenated = lm2_test_source_line(
        "p0:l1",
        1,
        "The conflict-of-laws framework is repeated.",
        1.0,
        false,
        None,
    );
    let authority = lm2_test_source_line(
        "p0:l2",
        2,
        "reliance on coercive au\u{0002}thority",
        1.0,
        false,
        None,
    );
    let statement = lm2_test_source_line(
        "p0:l3",
        3,
        "the state\u{0002}ment remains",
        1.0,
        false,
        None,
    );
    let conflict = lm2_test_source_line(
        "p0:l4",
        4,
        "a conflict\u{0002}of-laws analysis",
        1.0,
        false,
        None,
    );
    let abortion_rights = lm2_test_source_line(
        "p0:l5",
        5,
        "an abortion\u{0002}rights group would object",
        1.0,
        false,
        None,
    );
    let mut decoded = vec![
        (evidence_joined, Lm2Action::Keep),
        (evidence_hyphenated, Lm2Action::Keep),
        (authority, Lm2Action::Keep),
        (statement, Lm2Action::Keep),
        (conflict, Lm2Action::Keep),
        (abortion_rights, Lm2Action::Keep),
    ];

    assert_eq!(apply_document_discretionary_hyphen_repairs(&mut decoded), 4);
    assert_eq!(decoded[2].0.text, "reliance on coercive authority");
    // Document evidence overrides the broad `state-` compound fallback.
    assert_eq!(decoded[3].0.text, "the statement remains");
    assert_eq!(decoded[4].0.text, "a conflict-of-laws analysis");
    assert_eq!(decoded[5].0.text, "an abortion-rights group would object");
}

#[test]
fn terminal_pdf_hyphen_marker_remains_for_cross_line_joining() {
    let evidence = HashSet::new();
    let (text, repaired) = repair_internal_discretionary_hyphens("a conflict\u{0002}", &evidence);

    assert_eq!(repaired, 0);
    assert_eq!(text, "a conflict\u{0002}");
}

#[test]
fn document_spacing_repairs_require_repeated_phrase_or_narrow_note_cue() {
    let evidence_one = lm2_test_source_line(
        "p0:l0",
        0,
        "Choice of Model clauses govern the protocol.",
        1.0,
        false,
        None,
    );
    let evidence_two = lm2_test_source_line(
        "p0:l1",
        1,
        "Choice of Model clauses remain contestable.",
        1.0,
        false,
        None,
    );
    let fused = lm2_test_source_line(
        "p0:l2",
        2,
        "Choice of Modelclauses would specify a model.",
        1.0,
        false,
        None,
    );
    let weak_bigram = lm2_test_source_line(
        "p0:l3",
        3,
        "Some thing happened only once.",
        1.0,
        false,
        None,
    );
    let ordinary_word = lm2_test_source_line(
        "p0:l4",
        4,
        "Something remains an ordinary word.",
        1.0,
        false,
        None,
    );
    let mut note = lm2_test_source_line(
        "p0:l5",
        5,
        "161 SeeConstellation Power; but cf. KENNETH A.ADAMS,AMANUAL OF STYLE.",
        0.75,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.in_footnote_zone = true;
    note.below_footnote_divider = true;
    let mut decoded = vec![
        (evidence_one, Lm2Action::Keep),
        (evidence_two, Lm2Action::Keep),
        (fused, Lm2Action::Keep),
        (weak_bigram, Lm2Action::Keep),
        (ordinary_word, Lm2Action::Keep),
        (note, Lm2Action::Marginalia),
    ];

    assert!(apply_document_fused_word_spacing_repairs(&mut decoded) >= 5);
    assert!(decoded[2].0.text.contains("Model clauses"));
    assert!(decoded[4].0.text.contains("Something remains"));
    assert!(decoded[5].0.text.contains("See Constellation"));
    assert!(decoded[5].0.text.contains("A. ADAMS, A MANUAL OF STYLE"));
}

#[test]
fn append_line_preserves_url_and_scenario_hyphens() {
    let mut url = "https://example.test/qualify-as-".to_owned();
    append_line(&mut url, "sandwiches/.");
    assert_eq!(url, "https://example.test/qualify-as-sandwiches/.");

    let mut scenario = "The scenario-".to_owned();
    append_line(&mut scenario, "balanced result held.");
    assert_eq!(scenario, "The scenario-balanced result held.");
}

#[test]
fn assembly_attaches_standalone_body_marker_without_glue() {
    let mut body = lm2_test_source_line(
        "p0:l10",
        10,
        "Those contradictions are structural.",
        1.0,
        false,
        None,
    );
    body.page_width = 612.0;
    body.page_height = 792.0;
    body.left = 72.0;
    body.right = 300.0;
    body.bottom = 500.0;
    body.top = 512.0;
    body.font_height = 12.0;

    let mut marker = lm2_test_source_line("p0:l11", 11, "224", 0.5, false, None);
    marker.page_width = 612.0;
    marker.page_height = 792.0;
    marker.left = 304.0;
    marker.right = 318.0;
    marker.bottom = 507.0;
    marker.top = 514.0;
    marker.font_height = 6.0;

    let decoded = vec![(body, Lm2Action::Keep), (marker, Lm2Action::Keep)];
    let (_, blocks, sources) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        format!("Those contradictions are structural.{CALLOUT_START}224{CALLOUT_END}")
    );
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.as_deref())
            .collect::<Vec<_>>(),
        vec!["p0:l10", "p0:l11"]
    );
}

#[test]
fn same_row_leading_callout_moves_marker_back_and_rejoins_prose() {
    let mut previous = lm2_test_source_line(
        "p0:l24",
        24,
        "thereby triggering the right of first purchase for $10.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_width = 612.0;
    previous.page_height = 792.0;
    previous.left = 137.5;
    previous.right = 420.6;
    previous.bottom = 417.3;
    previous.top = 432.2;
    previous.font_height = 10.98;

    let mut marker_and_prose = lm2_test_source_line(
        "p0:l25",
        25,
        "346 At that",
        0.90,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    marker_and_prose.page_width = 612.0;
    marker_and_prose.page_height = 792.0;
    marker_and_prose.left = 420.6;
    marker_and_prose.right = 477.3;
    marker_and_prose.bottom = 417.8;
    marker_and_prose.top = 427.4;
    marker_and_prose.font_height = 9.85;
    marker_and_prose.in_footnote_zone = true;

    let continuation = lm2_test_source_line(
        "p0:l26",
        26,
        "point, the creditor would have a lien.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut note = lm2_test_source_line(
        "p0:l40",
        40,
        "346 Id. at 814.",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "346 At that point, the creditor would have a lien.".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![346];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&marker_and_prose, LiquidBlockRole::Paragraph),
                line_ref(&continuation, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![note_ref],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (marker_and_prose, Lm2Action::Keep),
        (continuation, Lm2Action::Keep),
        (note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_same_row_leading_callout_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        format!(
            "thereby triggering the right of first purchase for $10.{CALLOUT_START}346{CALLOUT_END} At that point, the creditor would have a lien."
        )
    );
    assert_eq!(sources[0].lines.len(), 3);
    assert_eq!(sources[1].block_index, 1);
}

#[test]
fn same_block_leading_callout_moves_marker_before_prose_fragment() {
    let mut previous = lm2_test_source_line(
        "p26:l1",
        1,
        "reason for augmenting discovery was to make litigation less expensive.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    previous.page_index = 26;
    previous.page_width = 612.0;
    previous.left = 108.0;
    previous.right = 455.64;
    previous.bottom = 668.796;
    previous.top = 684.96;
    previous.font_height = 12.0;

    let mut current = lm2_test_source_line(
        "p26:l2",
        2,
        "126 Today,",
        0.9,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    current.page_index = 26;
    current.page_width = 612.0;
    current.left = 456.24;
    current.right = 507.0;
    current.bottom = 668.796;
    current.top = 684.96;
    current.font_height = 9.63;

    let mut note = lm2_test_source_line(
        "p26:l22",
        22,
        "126 See generally the authority.",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 26;
    note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", previous.text, current.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![126];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&previous, LiquidBlockRole::Paragraph),
                line_ref(&current, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (current, Lm2Action::Keep),
        (note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_same_row_leading_callout_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert!(
        blocks[0]
            .text
            .contains(&format!("expensive.{CALLOUT_START}126{CALLOUT_END} Today,"))
    );
    assert_eq!(blocks.len(), 2);
}

#[test]
fn same_row_callout_bridge_joins_adjacent_body_paragraph() {
    let fixture = |continuation_left: f32| {
        let mut base = lm2_test_source_line(
            "p30:l1",
            1,
            "workers it represents.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        base.page_index = 30;
        base.page_width = 612.0;
        base.left = 57.96;
        base.right = 178.756;
        base.bottom = 700.0;
        base.top = 712.0;
        base.font_height = 12.0;
        let mut marker = lm2_test_source_line(
            "p30:l2",
            2,
            "196 Workers remain free to speak against the bargaining",
            1.0,
            false,
            None,
        );
        marker.page_index = 30;
        marker.page_width = 612.0;
        marker.left = 178.824;
        marker.right = 480.0;
        marker.bottom = 700.0;
        marker.top = 712.0;
        marker.font_height = 12.0;
        let mut continuation = lm2_test_source_line(
            "p30:l3",
            3,
            "positions of their unions.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        continuation.page_index = 30;
        continuation.page_width = 612.0;
        continuation.left = continuation_left;
        let mut stray = lm2_test_source_line("p30:l4", 4, "197", 0.7, false, None);
        stray.page_index = 30;
        let mut note = lm2_test_source_line(
            "p30:l20",
            20,
            "196 See the relevant authority.",
            0.75,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        note.page_index = 30;
        note.in_footnote_zone = true;
        let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
        note_ref.note_markers = vec![196];
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "{}{}{}{} Workers remain free to speak against the bargaining",
                    base.text, CALLOUT_START, 196, CALLOUT_END
                ),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: continuation.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Noise,
                text: stray.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: format!("{CALLOUT_START}196{CALLOUT_END} See the relevant authority."),
                label: None,
            },
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    line_ref(&base, LiquidBlockRole::Paragraph),
                    line_ref(&marker, LiquidBlockRole::Noise),
                ],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&continuation, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&stray, LiquidBlockRole::Noise)],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![note_ref],
            },
        ];
        let decoded = vec![
            (base, Lm2Action::Keep),
            (marker, Lm2Action::HideNoise),
            (continuation, Lm2Action::Keep),
            (stray, Lm2Action::HideNoise),
            (note, Lm2Action::Marginalia),
        ];
        (blocks, sources, decoded)
    };

    let (mut blocks, mut sources, decoded) = fixture(57.96);
    assert_eq!(
        apply_same_row_leading_callout_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 3);
    assert!(
        blocks[0]
            .text
            .contains("bargaining positions of their unions.")
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Noise);
    assert_eq!(blocks[1].text, "197");
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(sources[0].lines[1].role, LiquidBlockRole::Paragraph);

    let (mut blocks, mut sources, decoded) = fixture(120.0);
    assert_eq!(
        apply_same_row_leading_callout_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 4);
}

#[test]
fn markerless_table_row_is_not_absorbed_into_interleaved_note_flow() {
    let mut before = lm2_test_source_line(
        "p39:l40",
        40,
        "example, if the party argues for original meaning solely",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    before.page_index = 39;
    before.in_footnote_zone = true;
    let mut false_note = lm2_test_source_line(
        "p39:l41",
        41,
        "on the basis of Federalist No. 10, the court may not",
        0.95,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    false_note.page_index = 39;
    false_note.in_footnote_zone = true;
    let mut after = lm2_test_source_line(
        "p39:l42",
        42,
        "consider the ratification debates or Federalist No. 12",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    after.page_index = 39;
    after.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", before.text, after.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: false_note.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&before, LiquidBlockRole::Paragraph),
                line_ref(&after, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&false_note, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (before, Lm2Action::Keep),
        (false_note, Lm2Action::Marginalia),
        (after, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert!(blocks[0].text.contains("ratification debates"));
    assert!(!blocks[1].text.contains("ratification debates"));
}

#[test]
fn interleaved_note_continuation_leaves_body_and_restores_table_tail() {
    let body = lm2_test_source_line(
        "p3:l21",
        21,
        "a civil remedy for constitutional and statutory",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut misplaced = lm2_test_source_line(
        "p3:l36",
        36,
        "bnews.org/item [https://perma.cc/AE2F",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    misplaced.font_ratio_page_ref = 0.93;
    misplaced.font_ratio_doc = 0.95;
    misplaced.in_footnote_zone = true;
    let mut note = lm2_test_source_line(
        "p3:l35",
        35,
        "3 Citation ending at https://capital-",
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.in_footnote_zone = true;
    let mut tail = lm2_test_source_line(
        "p3:l37",
        37,
        "-ELDB] and the rest of the citation.",
        0.82,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    tail.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", body.text, misplaced.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}3{CALLOUT_END}. Citation ending at https://capital-"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: tail.text.clone(),
            label: Some("table".to_owned()),
        },
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![3];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&body, LiquidBlockRole::Paragraph),
                line_ref(&misplaced, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&tail, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (body, Lm2Action::Keep),
        (note, Lm2Action::Marginalia),
        (misplaced, Lm2Action::Keep),
        (tail, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        2
    );
    assert!(!blocks[0].text.contains("bnews.org"));
    assert!(blocks[1].text.contains("bnews.org"));
    assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);
    assert!(blocks[2].label.is_none());
}

#[test]
fn cross_page_year_citation_table_returns_to_previous_note() {
    let mut previous_note = lm2_test_source_line(
        "p0:l40",
        40,
        "383 Consent decree, 35 F. Supp. 3d 788 (E.D. La.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.in_footnote_zone = true;
    let mut carryover = lm2_test_source_line(
        "p1:l27",
        27,
        "2013) (No. 12-1924); Consent Decree at 29.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    carryover.page_index = 1;
    carryover.in_footnote_zone = true;
    carryover.font_ratio_page_ref = 0.8;
    carryover.font_ratio_doc = 0.82;
    let mut next_note = lm2_test_source_line(
        "p1:l28",
        28,
        "384 See N.Y.C. Charter.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 1;
    next_note.in_footnote_zone = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{CALLOUT_START}383{CALLOUT_END}. Consent decree, 35 F. Supp. 3d 788 (E.D. La."
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: carryover.text.clone(),
            label: Some("table".to_owned()),
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}384{CALLOUT_END}. See N.Y.C. Charter."),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![383];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![384];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&carryover, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![next_ref],
        },
    ];
    let decoded = vec![
        (previous_note, Lm2Action::Marginalia),
        (carryover, Lm2Action::Marginalia),
        (next_note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert!(blocks[1].label.is_none());
}

#[test]
fn cross_page_footnote_segment_paragraph_returns_to_previous_note() {
    let mut previous_note = lm2_test_source_line(
        "p56:l46",
        46,
        "287 Press Release, Padilla, Blumenthal Intro-",
        0.83,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.page_index = 56;
    previous_note.in_footnote_zone = true;
    let mut first_carryover = lm2_test_source_line(
        "p57:l18",
        18,
        "duce Bill to Provide Victims of Abuse (Dec. 15,",
        0.80,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    first_carryover.page_index = 57;
    first_carryover.in_footnote_zone = true;
    first_carryover.segment_block_id = 2;
    first_carryover.segment_block_shape = "footnote".to_owned();
    first_carryover.segment_block_footnote_like = true;
    let mut carryover_two = lm2_test_source_line(
        "p57:l19",
        19,
        "2025), https://www.padilla.senate.gov/newsroom/press-releases/",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    carryover_two.page_index = 57;
    carryover_two.in_footnote_zone = true;
    carryover_two.segment_block_id = 2;
    carryover_two.segment_block_shape = "footnote".to_owned();
    carryover_two.segment_block_footnote_like = true;
    let mut carryover_three = carryover_two.clone();
    carryover_three.id = "p57:l20".to_owned();
    carryover_three.line_index = 20;
    carryover_three.text = "padilla-blumenthal-introduce-bill-to-provide-victims".to_owned();
    let mut carryover_four = carryover_two.clone();
    carryover_four.id = "p57:l21".to_owned();
    carryover_four.line_index = 21;
    carryover_four.text = "[https://perma.cc/3MG9-9HQB].".to_owned();
    let mut next_note = lm2_test_source_line(
        "p57:l22",
        22,
        "288 H.R. 6091, 119th Cong. (2025).",
        0.80,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    next_note.page_index = 57;
    next_note.in_footnote_zone = true;
    let mut later_note = next_note.clone();
    later_note.id = "p57:l23".to_owned();
    later_note.line_index = 23;
    later_note.text = "289 See sources cited supra note 288.".to_owned();

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{CALLOUT_START}287{CALLOUT_END} Press Release, Padilla, Blumenthal Intro-"
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: first_carryover.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!(
                "{} {} {}",
                carryover_two.text, carryover_three.text, carryover_four.text
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{CALLOUT_START}288{CALLOUT_END} H.R. 6091, 119th Cong. (2025). {CALLOUT_START}289{CALLOUT_END} See sources cited supra note 288."
            ),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![287];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![288];
    let mut later_ref = line_ref(&later_note, LiquidBlockRole::Marginalia);
    later_ref.note_markers = vec![289];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&first_carryover, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![
                line_ref(&carryover_two, LiquidBlockRole::Paragraph),
                line_ref(&carryover_three, LiquidBlockRole::Paragraph),
                line_ref(&carryover_four, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![next_ref, later_ref],
        },
    ];
    let decoded = vec![
        (previous_note, Lm2Action::Marginalia),
        (first_carryover, Lm2Action::Marginalia),
        (carryover_two, Lm2Action::Keep),
        (carryover_three, Lm2Action::Keep),
        (carryover_four, Lm2Action::Keep),
        (next_note, Lm2Action::Marginalia),
        (later_note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);
    assert!(blocks[2].label.is_none());
    assert!(
        sources[2]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Marginalia)
    );
    assert_eq!(blocks[3].role, LiquidBlockRole::Marginalia);
    assert_eq!(sources[3].lines[0].note_markers, vec![288]);
}

fn inverted_note_continuation_fixture(
    next_marker: u16,
) -> (
    Vec<LiquidBlock>,
    Vec<LiquidBlockSourceLines>,
    Vec<(DeepLiquidSourceLine, Lm2Action)>,
) {
    let mut head = lm2_test_source_line(
        "p78:l34",
        34,
        "341 The term modalities is associated with Bobbitt, who defined a",
        0.76,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    head.page_index = 78;
    head.font_ratio_page_ref = 0.76;
    head.font_ratio_page = 0.76;
    head.font_ratio_doc = 0.76;
    head.segment_block_id = 3;
    head.segment_block_shape = "body".to_owned();
    let mut candidate = lm2_test_source_line(
        "p78:l35",
        35,
        "modality in a technical sense as the way we characterize expression as",
        0.78,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    candidate.page_index = 78;
    candidate.font_ratio_page_ref = 0.78;
    candidate.font_ratio_page = 0.78;
    candidate.font_ratio_doc = 0.78;
    candidate.segment_block_id = 3;
    candidate.segment_block_shape = "body".to_owned();
    let mut tail = lm2_test_source_line(
        "p78:l36",
        36,
        "true. Bobbitt, supra note 118, at 11.",
        0.79,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    tail.page_index = 78;
    tail.in_footnote_zone = true;
    tail.segment_block_id = 4;
    tail.segment_block_shape = "footnote".to_owned();
    tail.segment_block_footnote_like = true;
    let mut following = tail.clone();
    following.id = "p78:l37".to_owned();
    following.line_index = 37;
    following.text = format!("{next_marker} Griffin, supra note 23, at 1753.");

    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: candidate.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{CALLOUT_START}341{CALLOUT_END} The term modalities is associated with Bobbitt, who defined a"
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "{} {CALLOUT_START}{next_marker}{CALLOUT_END} Griffin, supra note 23, at 1753.",
                tail.text
            ),
            label: None,
        },
    ];
    let mut head_ref = line_ref(&head, LiquidBlockRole::Marginalia);
    head_ref.note_markers = vec![341];
    let mut following_ref = line_ref(&following, LiquidBlockRole::Marginalia);
    following_ref.note_markers = vec![next_marker];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&candidate, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![head_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&tail, LiquidBlockRole::Marginalia), following_ref],
        },
    ];
    let decoded = vec![
        (head, Lm2Action::Marginalia),
        (candidate, Lm2Action::Keep),
        (tail, Lm2Action::Marginalia),
        (following, Lm2Action::Marginalia),
    ];
    (blocks, sources, decoded)
}

#[test]
fn inverted_small_font_note_continuation_returns_to_numbered_head() {
    let (mut blocks, mut sources, decoded) = inverted_note_continuation_fixture(342);

    assert_eq!(
        apply_inverted_numbered_note_continuation_reflow(&mut blocks, &mut sources, &decoded,),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Marginalia);
    assert!(
        blocks[0]
            .text
            .contains("Bobbitt, who defined a modality in a technical sense")
    );
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        vec![34, 35]
    );
    assert!(
        sources[0]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Marginalia)
    );
    assert_eq!(sources[1].lines[1].note_markers, vec![342]);
}

#[test]
fn inverted_small_font_note_continuation_requires_sequential_next_head() {
    let (mut blocks, mut sources, decoded) = inverted_note_continuation_fixture(343);

    assert_eq!(
        apply_inverted_numbered_note_continuation_reflow(&mut blocks, &mut sources, &decoded,),
        0
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[0].lines[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn cross_page_footnote_segment_paragraph_requires_sequential_boundary_heads() {
    let mut previous_note = lm2_test_source_line(
        "p0:l40",
        40,
        "287 Previous note.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.in_footnote_zone = true;
    let mut first = lm2_test_source_line(
        "p1:l18",
        18,
        "markerless continuation",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    first.page_index = 1;
    first.in_footnote_zone = true;
    first.segment_block_id = 2;
    first.segment_block_shape = "footnote".to_owned();
    first.segment_block_footnote_like = true;
    let mut paragraph = first.clone();
    paragraph.id = "p1:l19".to_owned();
    paragraph.line_index = 19;
    paragraph.text = "body-sized but footnote-segment text".to_owned();
    let mut nonsequential = first.clone();
    nonsequential.id = "p1:l20".to_owned();
    nonsequential.line_index = 20;
    nonsequential.text = "289 A nonsequential note.".to_owned();
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: previous_note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: first.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: paragraph.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: nonsequential.text.clone(),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![287];
    let mut nonsequential_ref = line_ref(&nonsequential, LiquidBlockRole::Marginalia);
    nonsequential_ref.note_markers = vec![289];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&first, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&paragraph, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![nonsequential_ref],
        },
    ];
    let decoded = vec![
        (previous_note, Lm2Action::Marginalia),
        (first, Lm2Action::Marginalia),
        (paragraph, Lm2Action::Keep),
        (nonsequential, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[2].lines[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn cross_page_footnote_segment_paragraph_requires_open_previous_note() {
    let mut previous_note = lm2_test_source_line(
        "p0:l40",
        40,
        "287 A complete previous note.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.in_footnote_zone = true;
    let mut first = lm2_test_source_line(
        "p1:l18",
        18,
        "markerless continuation-looking text",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    first.page_index = 1;
    first.in_footnote_zone = true;
    first.segment_block_id = 2;
    first.segment_block_shape = "footnote".to_owned();
    first.segment_block_footnote_like = true;
    let mut paragraph = first.clone();
    paragraph.id = "p1:l19".to_owned();
    paragraph.line_index = 19;
    paragraph.text = "body-sized but footnote-segment text".to_owned();
    let mut next_note = first.clone();
    next_note.id = "p1:l20".to_owned();
    next_note.line_index = 20;
    next_note.text = "288 The exactly sequential note.".to_owned();
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: previous_note.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: first.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: paragraph.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: next_note.text.clone(),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![287];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![288];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&first, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&paragraph, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![next_ref],
        },
    ];
    let decoded = vec![
        (previous_note, Lm2Action::Marginalia),
        (first, Lm2Action::Marginalia),
        (paragraph, Lm2Action::Keep),
        (next_note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[2].lines[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn cross_page_source_marginalia_paragraph_returns_without_open_hyphen() {
    let mut previous_note = lm2_test_source_line(
        "p14:l44",
        44,
        "83 Prior authority rests on the fact",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    previous_note.page_index = 14;
    previous_note.in_footnote_zone = true;
    let mut carryover_one = lm2_test_source_line(
        "p15:l21",
        21,
        "that neither side had supplied a contrary record",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    carryover_one.page_index = 15;
    carryover_one.in_footnote_zone = true;
    carryover_one.segment_block_id = 7;
    carryover_one.segment_block_shape = "footnote".to_owned();
    carryover_one.segment_block_footnote_like = true;
    let mut carryover_two = carryover_one.clone();
    carryover_two.id = "p15:l22".to_owned();
    carryover_two.line_index = 22;
    carryover_two.text = "or identified a basis for another conclusion.".to_owned();
    let mut next_note = carryover_one.clone();
    next_note.id = "p15:l23".to_owned();
    next_note.line_index = 23;
    next_note.text = "84 See the following authority.".to_owned();

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}83{CALLOUT_END} Prior authority rests on the fact"),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!("{} {}", carryover_one.text, carryover_two.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}84{CALLOUT_END} See the following authority."),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous_note, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![83];
    let mut next_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![84];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&carryover_one, LiquidBlockRole::Marginalia),
                line_ref(&carryover_two, LiquidBlockRole::Marginalia),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![next_ref],
        },
    ];
    let decoded = vec![
        (previous_note, Lm2Action::Marginalia),
        (carryover_one, Lm2Action::Marginalia),
        (carryover_two, Lm2Action::Marginalia),
        (next_note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_interleaved_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert!(
        sources[1]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Marginalia)
    );
}

#[test]
fn gapped_numbered_note_tail_reflow_orders_inverted_donors() {
    let fixture = |final_left: f32| {
        let mut note_head = lm2_test_source_line(
            "p15:l37",
            37,
            "61 The federal enforcement program depended on",
            0.78,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        note_head.page_index = 15;
        note_head.page_width = 612.0;
        note_head.in_footnote_zone = true;
        note_head.segment_block_id = 5;
        note_head.segment_block_shape = "footnote".to_owned();
        note_head.segment_block_footnote_like = true;
        let mut note_last = note_head.clone();
        note_last.id = "p15:l45".to_owned();
        note_last.line_index = 45;
        note_last.text = "state and local".to_owned();
        let mut candidate_first = note_head.clone();
        candidate_first.id = "p15:l46".to_owned();
        candidate_first.line_index = 46;
        candidate_first.text = "enforcement efforts, with relationships".to_owned();
        let mut donor = note_head.clone();
        donor.id = "p15:l47".to_owned();
        donor.line_index = 47;
        donor.text = "that reflected a practical accommodation of".to_owned();
        let mut candidate_final = lm2_test_source_line(
            "p15:l48",
            48,
            "cooperative federalism).",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        candidate_final.page_index = 15;
        candidate_final.page_width = 612.0;
        candidate_final.left = final_left;
        candidate_final.segment_block_id = 6;
        candidate_final.segment_block_shape = "body".to_owned();
        let mut head_ref = line_ref(&note_head, LiquidBlockRole::Marginalia);
        head_ref.note_markers = vec![61];
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!("{} {}", candidate_first.text, candidate_final.text),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: format!(
                    "{CALLOUT_START}61{CALLOUT_END} The federal enforcement program depended on state and local"
                ),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: donor.text.clone(),
                label: None,
            },
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    line_ref(&candidate_first, LiquidBlockRole::Paragraph),
                    line_ref(&candidate_final, LiquidBlockRole::Paragraph),
                ],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![head_ref, line_ref(&note_last, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&donor, LiquidBlockRole::Marginalia)],
            },
        ];
        let decoded = vec![
            (note_head, Lm2Action::Marginalia),
            (note_last, Lm2Action::Marginalia),
            (candidate_first, Lm2Action::Keep),
            (donor, Lm2Action::Marginalia),
            (candidate_final, Lm2Action::Keep),
        ];
        (blocks, sources, decoded)
    };

    let (mut blocks, mut sources, decoded) = fixture(0.1);
    assert_eq!(
        apply_gapped_numbered_note_tail_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.contains(
        "state and local enforcement efforts, with relationships that reflected a practical accommodation of cooperative federalism)."
    ));
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        vec![37, 45, 46, 47, 48]
    );

    let (mut blocks, mut sources, decoded) = fixture(120.0);
    assert_eq!(
        apply_gapped_numbered_note_tail_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 3);
}

#[test]
fn plain_numeric_inline_callout_rejoins_preceding_body() {
    let previous = lm2_test_source_line(
        "p50:l20",
        20,
        "widely condemned decisions such as Dred Scott,",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut inline = lm2_test_source_line(
        "p50:l21",
        21,
        "260 Plessy,",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    inline.in_footnote_zone = true;
    let mut definition = lm2_test_source_line(
        "p50:l40",
        40,
        "260. Dred Scott v. Sandford.",
        0.8,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    definition.in_footnote_zone = true;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: inline.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}260{CALLOUT_END}. Dred Scott v. Sandford."),
            label: None,
        },
    ];
    let mut inline_ref = line_ref(&inline, LiquidBlockRole::Marginalia);
    inline_ref.note_markers = vec![260];
    let mut definition_ref = line_ref(&definition, LiquidBlockRole::Marginalia);
    definition_ref.note_markers = vec![260];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![inline_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![definition_ref],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (inline, Lm2Action::Marginalia),
        (definition, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_inline_body_callout_marginalia_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains(&format!(
        "Dred Scott, {CALLOUT_START}260{CALLOUT_END} Plessy,"
    )));
}

#[test]
fn contiguous_indented_quote_fragments_rejoin_body_paragraph() {
    let source = |block_index, line_index, text: &str, role| LiquidBlockSourceLines {
        block_index,
        lines: vec![LiquidSourceLineRef {
            id: Some(format!("p40:l{line_index}")),
            page_index: 40,
            line_index,
            text: text.to_owned(),
            role,
            note_markers: Vec::new(),
        }],
    };
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "(1) that the manufacturer's packaging sufficed to convey a valid offer"
                .to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "of contract terms and purchase terms on the homeowners' behalf. The court later continued: Moreover, consumers accept the accompanying terms and".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "conditions.295".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "The court even said the expectation had been building.".to_owned(),
            label: None,
        },
    ];
    let mut sources = vec![
        source(0, 1, &blocks[0].text, LiquidBlockRole::Marginalia),
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                LiquidSourceLineRef {
                    id: Some("p40:l2".to_owned()),
                    page_index: 40,
                    line_index: 2,
                    text: "of contract terms and purchase terms on the homeowners' behalf."
                        .to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                },
                LiquidSourceLineRef {
                    id: Some("p40:l14".to_owned()),
                    page_index: 40,
                    line_index: 14,
                    text: "consumers accept the accompanying terms and".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                },
            ],
        },
        source(2, 15, &blocks[2].text, LiquidBlockRole::Marginalia),
        source(3, 16, &blocks[3].text, LiquidBlockRole::Paragraph),
    ];

    assert_eq!(
        apply_contiguous_body_marginalia_reflow(&mut blocks, &mut sources, &[]),
        2
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.starts_with("(1) that the manufacturer's"));
    assert!(blocks[0].text.ends_with("terms and conditions.295"));
    assert_eq!(sources[0].lines.len(), 4);
    assert_eq!(sources[1].block_index, 1);
}

#[test]
fn contiguous_footnote_zone_continuation_does_not_merge_into_body() {
    let mut body = lm2_test_source_line(
        "p43:l22",
        22,
        "a model focused especially on financial information and data.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 43;
    let mut continuation = lm2_test_source_line(
        "p43:l23",
        23,
        "emergent capabilities in GPT-4, focusing particularly on skills that could not be explained merely",
        0.7,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    continuation.page_index = 43;
    continuation.in_footnote_zone = true;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: body.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: continuation.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&continuation, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (body, Lm2Action::Keep),
        (continuation, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_contiguous_body_marginalia_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
}

#[test]
fn false_marginalia_note_head_rejoins_split_pdf_filename() {
    let previous = lm2_test_source_line(
        "p0:l40",
        40,
        "14 See https://example.org/archive/10-5-",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p0:l41",
        41,
        "15.pdf [https://perma.cc/ABCD-1234].",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p0:l42",
        42,
        "15 See U.C.C. section 4A-211.",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: false_head.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: next.text.clone(),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![14];
    let mut false_ref = line_ref(&false_head, LiquidBlockRole::Marginalia);
    false_ref.note_markers = vec![15];
    let mut next_ref = line_ref(&next, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![15];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![false_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![next_ref],
        },
    ];

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains("10-5-15.pdf"));
    assert!(sources[0].lines[1].note_markers.is_empty());
    assert_eq!(sources[1].block_index, 1);
    assert_eq!(sources[1].lines[0].note_markers, vec![15]);
}

#[test]
fn false_marginalia_note_head_rejoins_citation_page_and_year() {
    let previous = lm2_test_source_line(
        "p0:l40",
        40,
        "97 See 45 LAW REV.",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p0:l41",
        41,
        "99 (2020).",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p0:l42",
        42,
        "98 See U.C.C. section 4A-211.",
        0.65,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: false_head.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: next.text.clone(),
            label: None,
        },
    ];
    let mut previous_ref = line_ref(&previous, LiquidBlockRole::Marginalia);
    previous_ref.note_markers = vec![97];
    let mut false_ref = line_ref(&false_head, LiquidBlockRole::Marginalia);
    false_ref.note_markers = vec![99];
    let mut next_ref = line_ref(&next, LiquidBlockRole::Marginalia);
    next_ref.note_markers = vec![98];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![previous_ref],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![false_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![next_ref],
        },
    ];

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "97 See 45 LAW REV. 99 (2020).");
    assert!(sources[0].lines[1].note_markers.is_empty());
    assert_eq!(sources[1].lines[0].note_markers, vec![98]);
}

#[test]
fn false_marginalia_note_head_rejoins_compare_pincite() {
    let previous = lm2_test_source_line(
        "p46:l39",
        39,
        "380 See Ortiz v. Fibreboard Corp., 527 U.S. 815, 842 (1999), discussed in Nagareda, supra note 179, at",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p46:l40",
        40,
        "232. Compare FED. R. CIV. P. 23(b)(2).",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p46:l41",
        41,
        "381 See 28 U.S.C. § 2243.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = [&previous, &false_head, &next]
        .into_iter()
        .map(|line| LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: line.text.clone(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = [&previous, &false_head, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| {
            let mut line = line_ref(line, LiquidBlockRole::Marginalia);
            line.note_markers = vec![[380, 232, 381][block_index]];
            LiquidBlockSourceLines {
                block_index,
                lines: vec![line],
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains("at 232. Compare"));
    assert!(sources[0].lines[1].note_markers.is_empty());
    assert_eq!(sources[1].lines[0].note_markers, vec![381]);
}

#[test]
fn false_marginalia_note_head_rejoins_standalone_supra_pincite() {
    let previous = lm2_test_source_line(
        "p48:l29",
        29,
        "305 See Rave, supra note",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p48:l30",
        30,
        "285.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p48:l31",
        31,
        "306 See D. Theodore Rave, Two Problems.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = [&previous, &false_head, &next]
        .into_iter()
        .map(|line| LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: line.text.clone(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = [&previous, &false_head, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| {
            let mut line = line_ref(line, LiquidBlockRole::Marginalia);
            line.note_markers = vec![[305, 285, 306][block_index]];
            LiquidBlockSourceLines {
                block_index,
                lines: vec![line],
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "305 See Rave, supra note 285.");
    assert!(sources[0].lines[1].note_markers.is_empty());
    assert_eq!(sources[1].lines[0].note_markers, vec![306]);
}

#[test]
fn false_marginalia_note_head_rejoins_standalone_page_pincite() {
    let previous = lm2_test_source_line(
        "p7:l44",
        44,
        "49 Others argue against the exclusion. See Yang & Dobbie, supra note 7, at",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p7:l45",
        45,
        "351.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p7:l46",
        46,
        "50 See Mayson, supra note 3, at 2225.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = [&previous, &false_head, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| LiquidBlock {
            role: if block_index == 0 {
                LiquidBlockRole::Noise
            } else {
                LiquidBlockRole::Marginalia
            },
            text: line.text.clone(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = [&previous, &false_head, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| {
            let mut line = line_ref(line, LiquidBlockRole::Marginalia);
            line.note_markers = vec![[49, 351, 50][block_index]];
            LiquidBlockSourceLines {
                block_index,
                lines: vec![line],
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Marginalia);
    assert!(blocks[0].text.ends_with("at 351."));
    assert!(sources[0].lines[1].note_markers.is_empty());
    assert_eq!(sources[1].lines[0].note_markers, vec![50]);
}

#[test]
fn false_marginalia_note_head_rejoins_standalone_docket_number() {
    let previous = lm2_test_source_line(
        "p34:l28",
        28,
        "165. Order, Dkt. No.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let false_head = lm2_test_source_line(
        "p34:l29",
        29,
        "88.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let next = lm2_test_source_line(
        "p34:l30",
        30,
        "166. Per Curiam Order.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    let mut blocks = [&previous, &false_head, &next]
        .into_iter()
        .map(|line| LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: line.text.clone(),
            label: None,
        })
        .collect::<Vec<_>>();
    let mut sources = [&previous, &false_head, &next]
        .into_iter()
        .enumerate()
        .map(|(block_index, line)| {
            let mut line = line_ref(line, LiquidBlockRole::Marginalia);
            line.note_markers = vec![[165, 88, 166][block_index]];
            LiquidBlockSourceLines {
                block_index,
                lines: vec![line],
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        apply_false_marginalia_note_head_reflow(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks[0].text, "165. Order, Dkt. No. 88.");
    assert_eq!(blocks.len(), 2);
}

#[test]
fn embedded_note_heads_reject_terminal_clause_and_docket_numbers() {
    assert_eq!(
        embedded_numeric_note_head_markers(
            "1. U.S. CONST. art. III, § 2, cl. 2. 2. See infra subpart I(C)."
        ),
        vec![1, 2]
    );
    assert_eq!(
        embedded_numeric_note_head_markers("178. Order, Dkt. No. 176."),
        vec![178]
    );
    assert_eq!(
        embedded_numeric_note_head_markers("19. Nineteen. 20. Twenty."),
        vec![19, 20]
    );
    assert_eq!(
        embedded_numeric_note_head_markers(&format!(
            "(2024) (speech). {CALLOUT_START}3{CALLOUT_END} Marc O. DeGirolami"
        )),
        vec![3]
    );
}

#[test]
fn sentineled_note_pair_recovers_trailing_page_note_zone() {
    let pair = format!(
        "{CALLOUT_START}1{CALLOUT_END} First authority. {CALLOUT_START}2{CALLOUT_END} Second authority."
    );
    let later = format!(
        "Second note continues. {CALLOUT_START}3{CALLOUT_END} Third authority. {CALLOUT_START}4{CALLOUT_END} Fourth authority."
    );
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: pair.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "citation continuation".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: later.clone(),
            label: Some("Table/Figure".to_owned()),
        },
    ];
    let make_ref = |block_index, line_index, text: String, role| LiquidBlockSourceLines {
        block_index,
        lines: vec![LiquidSourceLineRef {
            id: Some(format!("p2:l{line_index}")),
            page_index: 2,
            line_index,
            text,
            role,
            note_markers: Vec::new(),
        }],
    };
    let mut sources = vec![
        make_ref(0, 29, pair, LiquidBlockRole::Marginalia),
        make_ref(
            1,
            30,
            "citation continuation".to_owned(),
            LiquidBlockRole::Paragraph,
        ),
        make_ref(2, 31, later, LiquidBlockRole::Paragraph),
    ];

    assert_eq!(
        apply_sentineled_marginalia_page_zone_recovery(&mut blocks, &mut sources),
        3
    );
    assert!(
        blocks
            .iter()
            .all(|block| block.role == LiquidBlockRole::Marginalia)
    );
    assert_eq!(sources[0].lines[0].note_markers, vec![1, 2]);
    assert_eq!(sources[2].lines[0].note_markers, vec![3, 4]);
}

#[test]
fn sentineled_page_zone_enriches_carryover_and_provenance_noise_blocks() {
    let pair =
        format!("{CALLOUT_START}49{CALLOUT_END} First. {CALLOUT_START}50{CALLOUT_END} Second.");
    let carryover = format!("citation continues. {CALLOUT_START}51{CALLOUT_END} Third note.");
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: pair.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: carryover.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: "45. Hidden definition.".to_owned(),
            label: None,
        },
    ];
    let make_ref = |line_index, text: String, note_markers| LiquidSourceLineRef {
        id: Some(format!("p14:l{line_index}")),
        page_index: 14,
        line_index,
        text,
        role: LiquidBlockRole::Marginalia,
        note_markers,
    };
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![make_ref(29, pair, vec![])],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                make_ref(28, "earlier continuation".to_owned(), vec![]),
                make_ref(31, carryover, vec![]),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![make_ref(32, "45. Hidden definition.".to_owned(), vec![45])],
        },
    ];

    assert_eq!(
        apply_sentineled_marginalia_page_zone_recovery(&mut blocks, &mut sources),
        1
    );
    assert_eq!(sources[1].lines[1].note_markers, vec![51]);
    assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);
}

#[test]
fn sequential_note_head_inside_prior_marginalia_block_is_recovered() {
    let make_ref = |line_index, text: &str, note_markers| LiquidSourceLineRef {
        id: Some(format!("p68:l{line_index}")),
        page_index: 68,
        line_index,
        text: text.to_owned(),
        role: LiquidBlockRole::Marginalia,
        note_markers,
    };
    let mut source = LiquidBlockSourceLines {
        block_index: 682,
        lines: vec![
            make_ref(
                27,
                "308 See, e.g., Colleen McCullough, 164 U. PA. L.",
                vec![308],
            ),
            make_ref(
                28,
                "REV. 779, 801 (2016) (suggesting that contracts",
                vec![],
            ),
            make_ref(
                29,
                "costs avoided exceed the value of the vindicated rights).",
                vec![],
            ),
            make_ref(
                30,
                "309 350 F.2d 445 (1965) (invalidating a contract term",
                vec![],
            ),
            make_ref(
                31,
                "installment contracts to all items purchased so far",
                vec![],
            ),
        ],
    };

    assert_eq!(enrich_sequential_note_heads_within_block(&mut source), 1);
    assert_eq!(source.lines[3].note_markers, vec![309]);
    assert!(source.lines[1].note_markers.is_empty());
    assert!(source.lines[2].note_markers.is_empty());
    assert!(source.lines[4].note_markers.is_empty());
}

#[test]
fn sequential_note_head_recovery_rejects_nonterminal_citation_continuation() {
    let mut source = LiquidBlockSourceLines {
        block_index: 10,
        lines: vec![
            LiquidSourceLineRef {
                id: Some("p1:l10".to_owned()),
                page_index: 1,
                line_index: 10,
                text: "308 See a discussion in volume".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![308],
            },
            LiquidSourceLineRef {
                id: Some("p1:l11".to_owned()),
                page_index: 1,
                line_index: 11,
                text: "309 F.3d 400, 410 (2020).".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![],
            },
        ],
    };

    assert_eq!(enrich_sequential_note_heads_within_block(&mut source), 0);
    assert!(source.lines[1].note_markers.is_empty());
}

#[test]
fn sequential_note_head_recovery_supports_notes_above_five_hundred() {
    let mut source = LiquidBlockSourceLines {
        block_index: 939,
        lines: vec![
            LiquidSourceLineRef {
                id: Some("p71:l50".to_owned()),
                page_index: 71,
                line_index: 50,
                text: "500 Crecelius v. Smith, 125 N.W.2d 786 (Iowa 1964).".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![500],
            },
            LiquidSourceLineRef {
                id: Some("p71:l51".to_owned()),
                page_index: 71,
                line_index: 51,
                text: "501 958 N.W.2d 842 (Iowa 2021).".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![],
            },
            LiquidSourceLineRef {
                id: Some("p71:l52".to_owned()),
                page_index: 71,
                line_index: 52,
                text: "502 Id. at 845.".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![],
            },
        ],
    };

    assert_eq!(enrich_sequential_note_heads_within_block(&mut source), 2);
    assert_eq!(source.lines[1].note_markers, vec![501]);
    assert_eq!(source.lines[2].note_markers, vec![502]);
    assert_eq!(leading_numeric_token_marker("999 Last note."), Some(999));
    assert_eq!(leading_numeric_token_marker("1000 Not a note."), None);
}

#[test]
fn assembly_attached_marker_does_not_force_next_line_to_marker_geometry() {
    let mut body = lm2_test_source_line("p0:l10", 10, "The policy was tailored.", 1.0, false, None);
    body.page_width = 612.0;
    body.page_height = 792.0;
    body.left = 72.0;
    body.right = 260.0;
    body.bottom = 500.0;
    body.top = 512.0;
    body.font_height = 12.0;

    let mut marker = lm2_test_source_line("p0:l11", 11, "15", 0.5, false, None);
    marker.page_width = 612.0;
    marker.page_height = 792.0;
    marker.left = 264.0;
    marker.right = 274.0;
    marker.bottom = 507.0;
    marker.top = 514.0;
    marker.font_height = 6.0;

    let mut next = lm2_test_source_line(
        "p0:l12",
        12,
        "The next paragraph starts independently.",
        1.0,
        false,
        None,
    );
    next.page_width = 612.0;
    next.page_height = 792.0;
    next.left = 84.0;
    next.right = 390.0;
    next.bottom = 450.0;
    next.top = 462.0;
    next.font_height = 12.0;

    let decoded = vec![
        (body, Lm2Action::Keep),
        (marker, Lm2Action::Keep),
        (next, Lm2Action::Keep),
    ];
    let (_, blocks, _) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        format!("The policy was tailored.{CALLOUT_START}15{CALLOUT_END}")
    );
    assert_eq!(blocks[1].text, "The next paragraph starts independently.");
}

#[test]
fn production_slug_boilerplate_is_hidden() {
    assert!(looks_like_production_slug_boilerplate(
        "SPERBER IN PRINTER PREP (DO NOT DELETE) 3/17/2026 10:46 AM"
    ));
    assert!(looks_like_production_slug_boilerplate(
        "Copyright 2026 by Austin Kruse Printed in U.S.A."
    ));
    assert!(looks_like_production_slug_boilerplate(
        "BIONDI IN PRINTER FINAL (Do Not Delete) 2/6/2023 11:09 AM"
    ));
}

#[test]
fn production_slug_boilerplate_does_not_hide_ordinary_text() {
    assert!(!looks_like_production_slug_boilerplate(
        "The Article was printed and distributed in the United States."
    ));
    assert!(!looks_like_production_slug_boilerplate(
        "Copyright law gives authors exclusive rights in original works."
    ));
    assert!(!looks_like_production_slug_boilerplate(
        "Do not delete evidence before the litigation hold is lifted."
    ));
}

#[test]
fn footnote_state_carries_across_page_until_body_resume() {
    let mut body = lm2_test_source_line(
        "p0:l0",
        0,
        "Ordinary body text continues here",
        1.0,
        false,
        None,
    );
    body.font_height = 12.0;
    body.bottom = 0.75;
    body.top = 0.77;

    let mut note = lm2_test_source_line(
        "p0:l1",
        1,
        "1 Footnote text begins below divider",
        0.75,
        false,
        None,
    );
    note.font_height = 9.0;
    note.bottom = 0.12;
    note.top = 0.14;
    note.below_footnote_divider = true;
    note.page_has_footnote_divider = true;

    let mut continuation = lm2_test_source_line(
        "p1:l0",
        0,
        "continues from the prior page",
        0.75,
        false,
        None,
    );
    continuation.page_index = 1;
    continuation.font_height = 9.0;
    continuation.bottom = 0.78;
    continuation.top = 0.80;

    let mut resume = lm2_test_source_line(
        "p1:l1",
        1,
        "The Article returns to body prose",
        1.0,
        false,
        None,
    );
    resume.page_index = 1;
    resume.font_height = 12.0;
    resume.bottom = 0.66;
    resume.top = 0.68;

    let mut lines = vec![body, note, continuation, resume];
    enrich_lm2_document_features(&mut lines);

    assert!(!lines[0].doc_footnote_state);
    assert!(lines[1].doc_footnote_state);
    assert!(!lines[1].doc_footnote_continuation);
    assert!(lines[2].doc_footnote_state);
    assert!(lines[2].doc_footnote_continuation);
    assert!(!lines[3].doc_footnote_state);
    assert!(!lines[3].doc_footnote_continuation);
}

#[test]
fn footnote_carryover_overlay_marks_open_previous_page_continuation() {
    let mut previous = lm2_test_source_line(
        "p0:l20",
        20,
        "12. This footnote continues with",
        0.82,
        false,
        None,
    );
    previous.page_width = 612.0;
    previous.page_height = 792.0;
    previous.page_index = 0;
    previous.doc_note_marker = 12;
    previous.font_height = 8.0;
    previous.doc_font_footnote_size = 8.0;
    previous.doc_font_body_z = 1.4;
    previous.doc_font_footnote_z = 0.0;

    let mut continuation = lm2_test_source_line(
        "p1:l0",
        0,
        "additional discussion of the cited authority",
        0.82,
        false,
        None,
    );
    continuation.page_width = 612.0;
    continuation.page_height = 792.0;
    continuation.page_index = 1;
    continuation.font_height = 8.0;
    continuation.font_ratio_page_ref = 0.82;
    continuation.doc_font_footnote_size = 8.0;
    continuation.doc_font_body_z = 1.4;
    continuation.doc_font_footnote_z = 0.0;

    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
    ];
    apply_footnote_carryover_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn footnote_carryover_overlay_stops_at_expected_next_marker() {
    let mut previous = lm2_test_source_line(
        "p0:l20",
        20,
        "5. This footnote continues with",
        0.82,
        false,
        None,
    );
    previous.page_width = 612.0;
    previous.page_height = 792.0;
    previous.page_index = 0;
    previous.doc_note_marker = 5;
    previous.font_height = 8.0;
    previous.doc_font_footnote_size = 8.0;
    previous.doc_font_body_z = 1.4;
    previous.doc_font_footnote_z = 0.0;

    let mut next_marker = lm2_test_source_line(
        "p1:l0",
        0,
        "6. The next note begins here.",
        0.82,
        false,
        None,
    );
    next_marker.page_width = 612.0;
    next_marker.page_height = 792.0;
    next_marker.page_index = 1;
    next_marker.doc_note_marker = 6;
    next_marker.font_height = 8.0;
    next_marker.font_ratio_page_ref = 0.82;
    next_marker.doc_font_footnote_size = 8.0;
    next_marker.doc_font_body_z = 1.4;
    next_marker.doc_font_footnote_z = 0.0;

    let mut small_body = lm2_test_source_line(
        "p1:l1",
        1,
        "small-font line after the next note marker",
        0.82,
        false,
        None,
    );
    small_body.page_width = 612.0;
    small_body.page_height = 792.0;
    small_body.page_index = 1;
    small_body.font_height = 8.0;
    small_body.font_ratio_page_ref = 0.82;
    small_body.doc_font_footnote_size = 8.0;
    small_body.doc_font_body_z = 1.4;
    small_body.doc_font_footnote_z = 0.0;

    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (next_marker, Lm2Action::Marginalia),
        (small_body, Lm2Action::Keep),
    ];
    apply_footnote_carryover_overlay(&mut decoded);

    assert_eq!(decoded[2].1, Lm2Action::Keep);
}

#[test]
fn repetition_features_mark_three_page_edge_fingerprints() {
    let mut header0 = lm2_test_source_line(
        "p0:l0",
        0,
        "2026] THE FUGITIVE SLAVE ACT",
        0.86,
        false,
        None,
    );
    let mut header1 = lm2_test_source_line(
        "p1:l0",
        0,
        "2027] THE FUGITIVE SLAVE ACT",
        0.86,
        false,
        None,
    );
    let mut header2 = lm2_test_source_line(
        "p2:l0",
        0,
        "2028] THE FUGITIVE SLAVE ACT",
        0.86,
        false,
        None,
    );
    let mut body = lm2_test_source_line(
        "p2:l1",
        1,
        "A non-edge repeated phrase should not be marked.",
        1.0,
        false,
        None,
    );

    header0.page_index = 0;
    header1.page_index = 1;
    header2.page_index = 2;
    for line in [&mut header0, &mut header1, &mut header2] {
        line.bottom = 0.90;
        line.top = 0.92;
    }
    body.page_index = 2;
    body.bottom = 0.40;
    body.top = 0.42;

    let mut lines = vec![header0, header1, header2, body];
    enrich_lm2_document_features(&mut lines);

    for line in lines.iter().take(3) {
        assert!(line.doc_repeated_edge_text);
        assert_eq!(line.doc_repeated_text_count, 3);
        assert!(line.doc_repeated_top_edge);
        assert!(!line.doc_repeated_bottom_edge);
        assert!(line.doc_repeated_numeric_pattern);
    }
    assert!(!lines[3].doc_repeated_edge_text);
}

#[test]
fn vertical_numeric_axis_marks_same_x_tick_stack() {
    let mut tick0 = lm2_test_source_line("p0:l0", 0, "0%", 1.0, false, None);
    let mut tick1 = lm2_test_source_line("p0:l1", 1, "10%", 1.0, false, None);
    let mut tick2 = lm2_test_source_line("p0:l2", 2, "20%", 1.0, false, None);
    let mut body_callout = lm2_test_source_line("p0:l3", 3, "7", 1.0, false, None);
    for (line, bottom) in [
        (&mut tick0, 0.20_f32),
        (&mut tick1, 0.34_f32),
        (&mut tick2, 0.48_f32),
    ] {
        line.left = 0.10;
        line.right = 0.14;
        line.bottom = bottom;
        line.top = bottom + 0.02;
    }
    body_callout.left = 0.50;
    body_callout.right = 0.53;
    body_callout.bottom = 0.42;
    body_callout.top = 0.44;

    let mut lines = vec![tick0, tick1, tick2, body_callout];
    enrich_lm2_document_features(&mut lines);

    for line in lines.iter().take(3) {
        assert!(line.doc_vertical_axis_like);
        assert!(line.doc_vertical_numeric_axis_like);
        assert!(!line.doc_vertical_short_text_axis_like);
    }
    assert!(!lines[3].doc_vertical_axis_like);
}

#[test]
fn numeric_catboost_features_include_table_numeric_cell_like() {
    let mut line = lm2_test_source_line("p0:l0", 0, "10%", 1.0, false, None);
    line.left = 0.12;
    line.right = 0.17;
    let features = lm2_numeric_catboost_features(&line);
    assert_eq!(features.get("table_numeric_cell_like").copied(), Some(1.0));
}

#[test]
fn table_column_feature_marks_repeated_grid_cells() {
    let mut lines = Vec::new();
    for row in 0..3 {
        for col in 0..2 {
            let mut line = lm2_test_source_line(
                &format!("p0:l{}", row * 2 + col),
                row * 2 + col,
                if col == 0 { "2019" } else { "$42" },
                1.0,
                false,
                None,
            );
            line.left = 0.20 + (col as f32 * 0.18);
            line.right = line.left + 0.06;
            line.bottom = 0.20 + (row as f32 * 0.08);
            line.top = line.bottom + 0.02;
            lines.push(line);
        }
    }

    enrich_lm2_document_features(&mut lines);

    assert!(lines.iter().all(|line| line.page_table_column_like));
    let features = lm2_numeric_catboost_features(&lines[0]);
    assert_eq!(features.get("page_table_column_like").copied(), Some(1.0));
}

#[test]
fn dotleader_context_marks_following_toc_line_without_dots() {
    let rows = [
        "Introduction ........................................ 1",
        "Background . . . . . . . . . . . . . . . . . . . . 3",
        "Methodology ....................................... 8",
        "Appendix A",
    ];
    let mut lines = rows
        .iter()
        .enumerate()
        .map(|(index, text)| {
            lm2_test_source_line(&format!("p0:l{}", index), index, text, 1.0, false, None)
        })
        .collect::<Vec<_>>();

    enrich_lm2_document_features(&mut lines);

    assert!(lines[1].prev_line_has_dotleader);
    assert_eq!(lines[3].prev4_dotleader_count, 2);
    assert_eq!(lines[3].prev4_spaced_dotleader_count, 3);
    assert_eq!(lines[3].prev4_strong_dotleader_count, 3);
    assert!(lines[3].prev4_toc_leader_context);
    let features = lm2_numeric_catboost_features(&lines[3]);
    assert_eq!(features.get("prev4_toc_leader_context").copied(), Some(1.0));
    assert_eq!(features.get("prev4_dotleader_count").copied(), Some(2.0));
}

#[test]
fn column_spacing_features_mark_numeric_table_line() {
    let line = lm2_test_source_line(
        "p0:l0",
        0,
        "White applicants     42%     37%     21%",
        1.0,
        false,
        None,
    );

    let features = lm2_numeric_catboost_features(&line);

    assert_eq!(features.get("internal_space_run_max").copied(), Some(5.0));
    assert_eq!(features.get("numeric_token_count").copied(), Some(3.0));
    assert_eq!(features.get("percent_token_count").copied(), Some(3.0));
    assert_eq!(
        features.get("has_large_internal_space_gap").copied(),
        Some(1.0)
    );
    assert_eq!(
        features.get("columnar_numeric_text_like").copied(),
        Some(1.0)
    );
}

#[test]
fn marker_continuity_marks_mid_sequence_next_page() {
    let mut note1 = lm2_test_source_line(
        "p0:l0",
        0,
        "1 First marker begins the notes",
        0.82,
        false,
        None,
    );
    let mut note2 = lm2_test_source_line(
        "p0:l1",
        1,
        "2 Second marker closes the page",
        0.82,
        false,
        None,
    );
    let mut note3 = lm2_test_source_line(
        "p1:l0",
        0,
        "3 Continued marker opens the next page",
        0.82,
        false,
        None,
    );
    let mut continuation = lm2_test_source_line(
        "p1:l1",
        1,
        "Continuation prose inherits the page marker signal",
        0.82,
        false,
        None,
    );
    note1.page_index = 0;
    note2.page_index = 0;
    note3.page_index = 1;
    continuation.page_index = 1;

    let mut lines = vec![note1, note2, note3, continuation];
    enrich_lm2_document_features(&mut lines);

    assert_eq!(lines[0].doc_note_marker, 1);
    assert!(lines[0].doc_note_marker_first_on_page);
    assert!(!lines[0].doc_note_marker_mid_sequence_page);
    assert_eq!(lines[1].doc_note_marker, 2);
    assert!(!lines[1].doc_note_marker_first_on_page);

    assert_eq!(lines[2].doc_note_marker, 3);
    assert!(lines[2].doc_note_marker_first_on_page);
    assert!(lines[2].doc_note_marker_mid_sequence_page);
    assert!(lines[2].doc_note_marker_follows_previous_page);
    assert_eq!(lines[2].doc_note_marker_page_delta, 1);

    assert_eq!(lines[3].doc_note_marker, 0);
    assert!(!lines[3].doc_note_marker_first_on_page);
    assert!(lines[3].doc_note_marker_mid_sequence_page);
    assert!(lines[3].doc_note_marker_follows_previous_page);
    assert_eq!(lines[3].doc_note_marker_page_delta, 1);
}

#[test]
fn filename_fallback_title_detection_is_narrow() {
    assert!(looks_like_filename_fallback_title(
        "duke_law_journal__DLJ_vol72_iss5_Building_Trusts.pdf"
    ));
    assert!(looks_like_filename_fallback_title(
        "northwestern_university_law_review__place_names"
    ));
    assert!(!looks_like_filename_fallback_title(
        "PLACE NAMES AND PRESIDENTIAL CONTROL"
    ));
    assert!(!looks_like_filename_fallback_title("Is Tax “Law”?"));
}

#[test]
fn recovered_title_can_complete_a_truncated_metadata_title() {
    assert!(lm2_recovered_title_is_better(
        "The UCC Drafting Process and Six Questions about Article 4A: Is There a Need for Revisions",
        "The UCC Drafting Process and Six Questions about Article 4A: Is"
    ));
}

#[test]
fn repository_title_can_correct_uppercase_display_ocr() {
    assert!(lm2_recovered_title_is_better(
        "The UCC Drafting Process and Six Questions about Article 4A: Is There a Need for Revisions to the Uniform Funds Transfers Law?",
        "THE UCC DRAFJTING PROCESS AND SIX QUESTIONS ABOUT ARTICLE 4A: IS THERE A NEED FOR REVISIONS TO THE UNIFORM FUNDS TRANSFERS LAW?"
    ));
}

#[test]
fn lm2_fallback_title_skips_generic_heading_labels() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "Notes".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "BUILDING TRUST(S): RETHINKING ASSET".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "RETURN IN KLEPTOCRACY FORFEITURES".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "ABSTRACT".to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some("BUILDING TRUST(S): RETHINKING ASSET RETURN IN KLEPTOCRACY FORFEITURES")
    );
}

#[test]
fn lm2_fallback_title_stops_before_author_heading() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "Article".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "Regulatory History and Judicial Review".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "Todd Phillips† and Anthony Moffa††".to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some("Regulatory History and Judicial Review")
    );
}

#[test]
fn lm2_fallback_title_prefers_leading_paragraph_over_author_title() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "RETHINKING ROBOT LIABILITY".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Title,
            text: "ZACHARY HENDERSON".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "INTRODUCTION................................................................480"
                .to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some("RETHINKING ROBOT LIABILITY")
    );
}

#[test]
fn lm2_fallback_title_accepts_long_caps_title_before_introduction() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "Note".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "Roger Cowie".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: "Copyright (c) 1992; Roger Cowie".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "CANCELLATION OF WIRE TRANSFERS UNDER ARTICLE 4A OF THE UNIFORM COMMERCIAL CODE: DELBRUECK REVISITED".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "I. Introduction".to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some(
            "CANCELLATION OF WIRE TRANSFERS UNDER ARTICLE 4A OF THE UNIFORM COMMERCIAL CODE: DELBRUECK REVISITED"
        )
    );
}

#[test]
fn lm2_fallback_title_stops_before_toc_dotleaders() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "EQUITABLE REGULATORY BALANCING".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "INTRODUCTION................................................................532"
                .to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "B. In Search of Preliminary Relief in Regulatory Cases".to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some("EQUITABLE REGULATORY BALANCING")
    );
}

#[test]
fn lm2_fallback_title_ignores_later_citation_fragments() {
    let blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "CORPORATE GOODWILL".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "INTRODUCTION................................................................587"
                .to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Title,
            text: "Esty & Quentin Karpilow, Harnessing Investor Interest in Sustainability"
                .to_owned(),
            label: None,
        },
    ];

    assert_eq!(
        lm2_fallback_title_from_blocks(&blocks).as_deref(),
        Some("CORPORATE GOODWILL")
    );
}

#[test]
fn lm2_decoder_constants_can_change_sequence_choice() {
    let lines = vec![
        lm2_test_source_line("p0:l0", 0, "Ordinary body text", 1.0, false, None),
        lm2_test_source_line("p0:l1", 1, "continues on the next line", 1.0, false, None),
    ];
    let base_runtime = Lm2Runtime {
        model_label: "test".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: None,
        pp_footnote_region_membership: false,
        marker_decoder_prior: false,
        small_font_decoder_prior: false,
        small_font_sequence_prior: false,
        anchored_marginalia_flow_guard: false,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: None,
        }),
    };
    let base = decode_page(&base_runtime, &lines);
    assert_eq!(base[0].1, Lm2Action::Keep);
    assert_eq!(base[1].1, Lm2Action::Keep);

    let mut weights = HashMap::new();
    weights.insert("start_arc:marginalia".to_owned(), 4.0);
    weights.insert("transition_arc:marginalia->marginalia".to_owned(), 4.0);
    let fitted_runtime = Lm2Runtime {
        model_label: "test-fitted".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: None,
        pp_footnote_region_membership: false,
        marker_decoder_prior: false,
        small_font_decoder_prior: false,
        small_font_sequence_prior: false,
        anchored_marginalia_flow_guard: false,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test-fitted".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: Some(Lm2DecoderConstants { weights }),
        }),
    };
    let fitted = decode_page(&fitted_runtime, &lines);
    assert_eq!(fitted[0].1, Lm2Action::Marginalia);
    assert_eq!(fitted[1].1, Lm2Action::Marginalia);
}

#[test]
fn marker_decoder_prior_is_off_by_default() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "continuing discussion, see id.",
        0.82,
        false,
        None,
    );
    line.bottom = 0.30;
    line.top = 0.32;
    line.doc_note_marker_mid_sequence_page = true;
    line.doc_note_marker_follows_previous_page = true;

    assert!(marker_continuity_decoder_prior_eligible(&line));
    let decoded = decode_page(&lm2_zero_runtime(false, false, false), &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn marker_decoder_prior_can_recover_supported_continuation() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "continuing discussion, see id.",
        0.82,
        false,
        None,
    );
    line.bottom = 0.30;
    line.top = 0.32;
    line.doc_note_marker_mid_sequence_page = true;
    line.doc_note_marker_follows_previous_page = true;

    let decoded = decode_page(&lm2_zero_runtime(true, false, false), &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
}

#[test]
fn marker_decoder_prior_rejects_noise_shapes() {
    let mut header =
        lm2_test_source_line("p0:l0", 0, "2026] Law Review Vol. 44", 0.82, false, None);
    header.doc_note_marker_mid_sequence_page = true;
    header.doc_note_marker_follows_previous_page = true;
    assert!(!marker_continuity_decoder_prior_eligible(&header));

    let mut table = lm2_test_source_line(
        "p0:l1",
        1,
        "continuing discussion, see id.",
        0.82,
        false,
        Some(LiquidBlockRole::Table),
    );
    table.bottom = 0.30;
    table.top = 0.32;
    table.doc_note_marker_mid_sequence_page = true;
    table.doc_note_marker_follows_previous_page = true;
    assert!(!marker_continuity_decoder_prior_eligible(&table));
}

#[test]
fn small_font_decoder_prior_is_off_by_default() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "source-tail footnote prose continues here, see id.",
        0.82,
        false,
        None,
    );
    line.font_ratio_doc = 0.82;
    line.bottom = 0.30;
    line.top = 0.32;

    assert!(small_font_lower_page_decoder_prior_eligible(&line));
    let decoded = decode_page(&lm2_zero_runtime(false, false, false), &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn small_font_decoder_prior_can_recover_lower_page_note_prose() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "source-tail footnote prose continues here, see id.",
        0.82,
        false,
        None,
    );
    line.font_ratio_doc = 0.82;
    line.bottom = 0.30;
    line.top = 0.32;

    let decoded = decode_page(&lm2_zero_runtime(false, true, false), &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
}

#[test]
fn small_font_decoder_prior_rejects_upper_page_and_title_like_shapes() {
    let mut upper = lm2_test_source_line(
        "p0:l0",
        0,
        "source-tail footnote prose continues here, see id.",
        0.82,
        false,
        None,
    );
    upper.font_ratio_doc = 0.82;
    upper.bottom = 0.66;
    upper.top = 0.68;
    assert!(!small_font_lower_page_decoder_prior_eligible(&upper));

    let mut heading =
        lm2_test_source_line("p0:l1", 1, "SUPERIOR OR OTHER OFFICER", 0.82, false, None);
    heading.font_ratio_doc = 0.82;
    heading.bottom = 0.30;
    heading.top = 0.32;
    assert!(!small_font_lower_page_decoder_prior_eligible(&heading));
}

#[test]
fn small_font_decoder_prior_rejects_page_furniture_boilerplate() {
    for text in [
        "[Vol. 46",
        "Page 12",
        "Published by Scholar Commons, 1976",
        "This Article is brought to you for free and open access by FLASH",
        "Fordham Law Archive of Scholarship and History",
        "For more information, please contact tmelnick@law.fordham.edu.",
        "https://scholarcommons.sc.edu/sclr/vol5/iss3/6",
        "© 2024 University Press – All rights reserved",
        "footnote continued on next page",
    ] {
        let mut line = lm2_test_source_line("p0:l0", 0, text, 0.82, false, None);
        line.font_ratio_doc = 0.82;
        line.bottom = 0.30;
        line.top = 0.32;
        assert!(
            !small_font_lower_page_decoder_prior_eligible(&line),
            "{text}"
        );
    }

    let mut note = lm2_test_source_line(
        "p0:l1",
        1,
        "source-tail footnote prose continues here, see id.",
        0.82,
        false,
        None,
    );
    note.font_ratio_doc = 0.82;
    note.bottom = 0.30;
    note.top = 0.32;
    assert!(small_font_lower_page_decoder_prior_eligible(&note));
}

#[test]
fn small_font_decoder_prior_requires_note_evidence() {
    let mut ordinary = lm2_test_source_line(
        "p0:l0",
        0,
        "source-tail prose continues here without a note cue",
        0.82,
        false,
        None,
    );
    ordinary.font_ratio_doc = 0.82;
    ordinary.bottom = 0.30;
    ordinary.top = 0.32;
    assert!(!small_font_lower_page_decoder_prior_eligible(&ordinary));

    let mut note_start = lm2_test_source_line(
        "p0:l1",
        1,
        "17 Supported note text begins",
        0.82,
        false,
        None,
    );
    note_start.font_ratio_doc = 0.82;
    note_start.bottom = 0.30;
    note_start.top = 0.32;
    assert!(small_font_lower_page_decoder_prior_eligible(&note_start));
}

#[test]
fn small_font_sequence_prior_is_transition_only() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "continuing discussion, see id.",
        0.82,
        false,
        None,
    );
    line.font_ratio_doc = 0.82;
    line.bottom = 0.30;
    line.top = 0.32;

    assert!(small_font_lower_page_decoder_prior_eligible(&line));
    assert_eq!(
        small_font_sequence_continuation_prior(&line, Lm2Action::Keep, Lm2Action::Marginalia),
        0.0
    );
    assert!(
        small_font_sequence_continuation_prior(&line, Lm2Action::Marginalia, Lm2Action::Marginalia)
            > 0.0
    );

    let decoded = decode_page(&lm2_zero_runtime(false, false, true), &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn anchored_marginalia_flow_guard_caps_unanchored_continuations() {
    let lines = vec![
        lm2_test_source_line("p0:l0", 0, "1. See id.", 0.82, false, None),
        lm2_test_source_line(
            "p0:l1",
            1,
            "continued prose without an anchor",
            0.82,
            false,
            None,
        ),
        lm2_test_source_line("p0:l2", 2, "more continued prose", 0.82, false, None),
        lm2_test_source_line(
            "p0:l3",
            3,
            "third unanchored continuation",
            0.82,
            false,
            None,
        ),
    ];
    let mut path = vec![
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
    ];

    apply_anchored_marginalia_flow_guard(&lines, &mut path);

    assert_eq!(
        path,
        vec![
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Keep,
        ]
    );
}

#[test]
fn anchored_marginalia_flow_guard_rejects_unanchored_starts() {
    let lines = vec![
        lm2_test_source_line("p0:l0", 0, "ordinary small text", 0.82, false, None),
        lm2_test_source_line("p0:l1", 1, "still no note anchor", 0.82, false, None),
        lm2_test_source_line("p0:l2", 2, "2. Anchored note starts", 0.82, false, None),
        lm2_test_source_line("p0:l3", 3, "allowed continuation", 0.82, false, None),
    ];
    let mut path = vec![
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
        Lm2Action::Marginalia,
    ];

    apply_anchored_marginalia_flow_guard(&lines, &mut path);

    assert_eq!(
        path,
        vec![
            Lm2Action::Keep,
            Lm2Action::Keep,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
        ]
    );
}

#[test]
fn anchored_marginalia_flow_guard_applies_after_decode() {
    let lines = vec![
        lm2_test_source_line("p0:l0", 0, "1. See id.", 0.82, false, None),
        lm2_test_source_line("p0:l1", 1, "first continuation", 0.82, false, None),
        lm2_test_source_line("p0:l2", 2, "second continuation", 0.82, false, None),
        lm2_test_source_line("p0:l3", 3, "third continuation", 0.82, false, None),
    ];
    let mut weights = HashMap::new();
    weights.insert("start_arc:marginalia".to_owned(), 4.0);
    weights.insert("transition_arc:marginalia->marginalia".to_owned(), 4.0);
    let runtime = Lm2Runtime {
        model_label: "test-flow-guard".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: None,
        pp_footnote_region_membership: false,
        marker_decoder_prior: false,
        small_font_decoder_prior: false,
        small_font_sequence_prior: false,
        anchored_marginalia_flow_guard: true,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test-flow-guard".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: Some(Lm2DecoderConstants { weights }),
        }),
    };

    let decoded = decode_page(&runtime, &lines)
        .into_iter()
        .map(|(_, action)| action)
        .collect::<Vec<_>>();

    assert_eq!(
        decoded,
        vec![
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Keep,
        ]
    );
}

#[test]
fn guarded_pp_prior_rejects_numeric_footnote_rows() {
    assert!(guarded_pp_prior_action(
        Some("marginalia"),
        "footnote",
        "footnote",
        0.80,
        "Some real note text"
    ));
    assert!(!guarded_pp_prior_action(
        Some("marginalia"),
        "footnote",
        "footnote",
        0.799,
        "Some real note text"
    ));
    assert!(guarded_pp_prior_action(
        Some("marginalia"),
        "footnote",
        "footnote",
        0.90,
        "Some real note text"
    ));
    assert!(!guarded_pp_prior_action(
        Some("marginalia"),
        "footnote",
        "footnote",
        0.90,
        "775"
    ));
    assert!(guarded_pp_prior_action(
        Some("hide_noise"),
        "table",
        "table",
        0.75,
        "table row"
    ));
}

#[test]
fn pp_footnote_prior_can_change_sequence_choice() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "ordinary looking note prose", 1.0, false, None);
    let mut rows = HashMap::new();
    rows.insert(
        pp_prior_key("doc.pdf", line.page_index, line.line_index, &line.text),
        Lm2PpPrior {
            role: "footnote".to_owned(),
            label: "footnote".to_owned(),
            score: 0.92,
        },
    );
    let runtime = Lm2Runtime {
        model_label: "test-pp".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: Some(Lm2PpPriorIndex {
            source: PathBuf::from("pp.jsonl"),
            rows,
        }),
        pp_footnote_region_membership: false,
        marker_decoder_prior: false,
        small_font_decoder_prior: false,
        small_font_sequence_prior: false,
        anchored_marginalia_flow_guard: false,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test-pp".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: None,
        }),
    };
    annotate_pp_prior(&runtime, "doc.pdf", &mut line);
    assert_eq!(line.pp_prior_role.as_deref(), Some("footnote"));
    let decoded = decode_page(&runtime, &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
}

#[test]
fn pp_footnote_membership_override_is_post_decode_only() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 1.0, false, None);
    line.bottom = 0.30;
    line.top = 0.32;
    line.font_ratio_doc = 0.88;
    line.font_ratio_page = 0.88;
    line.pp_prior_role = Some("footnote".to_owned());
    line.pp_prior_label = Some("footnote".to_owned());
    line.pp_prior_score = Some(0.92);
    let runtime = Lm2Runtime {
        model_label: "test-pp-membership".to_owned(),
        load_warnings: Vec::new(),
        pp_priors: None,
        pp_footnote_region_membership: true,
        marker_decoder_prior: false,
        small_font_decoder_prior: false,
        small_font_sequence_prior: false,
        anchored_marginalia_flow_guard: false,
        body_preservation_guard: false,
        action_neutral_blocksplit: false,
        toc_overlay: false,
        front_matter_guard: false,
        marginalia_preservation_guard: false,
        start_score_scale: 1.0,
        transition_score_scale: 1.0,
        fasttab_model: None,
        native_catboost_model: None,
        context_twopass_model: None,
        context_arbiter_model: None,
        note_head_model: None,
        link_ranker_model: None,
        numeric_catboost_model: None,
        static_front_overlay: None,
        model: Some(Lm2Model {
            model_id: "test-pp-membership".to_owned(),
            model_type: "hashed_softmax_action_v1".to_owned(),
            actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
            feature_dim: 1,
            bias: vec![0.0, 0.0, 0.0],
            weights: vec![vec![0.0], vec![0.0], vec![0.0]],
            feature_schema: None,
            decoder_constants: None,
        }),
    };

    let decoded = decode_page(&runtime, &[line]);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
}

#[test]
fn pp_footnote_membership_rejects_numeric_rows() {
    let mut line = lm2_test_source_line("p0:l0", 0, "775", 1.0, false, None);
    mark_pp_footnote(&mut line);
    assert!(!pp_footnote_region_member(&line));
}

#[test]
fn pp_footnote_membership_accepts_region_member_without_geometry() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "ordinary looking body prose", 1.0, false, None);
    mark_pp_footnote(&mut line);
    let lines = vec![line];
    let mut path = vec![Lm2Action::Keep];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_accepts_score_below_high_confidence_cutoff() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
    line.bottom = 0.30;
    line.top = 0.32;
    line.font_ratio_doc = 0.88;
    mark_pp_footnote(&mut line);
    line.pp_prior_score = Some(0.89);
    let lines = vec![line];
    let mut path = vec![Lm2Action::Keep];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_accepts_score_at_promotion_threshold() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
    mark_pp_footnote(&mut line);
    line.pp_prior_score = Some(0.80);

    assert!(pp_footnote_region_member(&line));
}

#[test]
fn pp_footnote_membership_rejects_score_below_promotion_threshold() {
    let mut line =
        lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
    mark_pp_footnote(&mut line);
    line.pp_prior_score = Some(0.799);

    assert!(!pp_footnote_region_member(&line));
}

#[test]
fn pp_footnote_membership_forward_closure_rejects_furniture_rows() {
    let mut seed = lm2_test_source_line(
        "p0:l0",
        0,
        "12. See United States v. Alvarez",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let mut furniture = lm2_test_source_line("p0:l1", 1, "Page 12", 0.82, false, None);
    furniture.bottom = 0.28;
    furniture.top = 0.30;
    let lines = vec![seed, furniture];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Keep]);
}

#[test]
fn pp_footnote_membership_backfills_from_region_member_to_note_start() {
    let mut start = lm2_test_source_line(
        "p0:l0",
        0,
        "9. Steve Quinn, Congress and the States",
        0.98,
        false,
        None,
    );
    start.bottom = 0.34;
    start.top = 0.36;
    let mut continuation = lm2_test_source_line(
        "p0:l1",
        1,
        "standing are beyond the scope of this example",
        0.98,
        false,
        None,
    );
    continuation.bottom = 0.32;
    continuation.top = 0.34;
    let mut seed = lm2_test_source_line(
        "p0:l2",
        2,
        "states have standing under the rule",
        0.88,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    seed.font_ratio_doc = 0.88;
    mark_pp_footnote(&mut seed);
    let lines = vec![start, continuation, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(
        path,
        vec![
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
        ]
    );
}

#[test]
fn pp_footnote_membership_does_not_backward_close_non_start_continuation() {
    let mut first = lm2_test_source_line(
        "p0:l0",
        0,
        "customers goods there exposed as illustrations, see id.",
        0.82,
        false,
        None,
    );
    first.bottom = 0.34;
    first.top = 0.36;
    first.font_ratio_doc = 0.82;
    let mut second = lm2_test_source_line(
        "p0:l1",
        1,
        "the legal rule discussed below, see id.",
        0.82,
        false,
        None,
    );
    second.bottom = 0.32;
    second.top = 0.34;
    second.font_ratio_doc = 0.82;
    let mut seed = lm2_test_source_line(
        "p0:l2",
        2,
        "mark cases later cited the same passage",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let lines = vec![first, second, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(
        path,
        vec![Lm2Action::Keep, Lm2Action::Keep, Lm2Action::Marginalia,]
    );
}

#[test]
fn pp_footnote_membership_rejects_geometry_only_backward_continuation() {
    let mut geometry_only = lm2_test_source_line(
        "p0:l0",
        0,
        "ordinary small font paragraph text before the note",
        0.82,
        false,
        None,
    );
    geometry_only.bottom = 0.34;
    geometry_only.top = 0.36;
    geometry_only.font_ratio_doc = 0.82;
    let mut seed = lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let lines = vec![geometry_only, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_rejects_page_divider_only_backward_continuation() {
    let mut page_divider_only = lm2_test_source_line(
        "p0:l0",
        0,
        "ordinary paragraph text above the page footnote divider",
        1.0,
        false,
        None,
    );
    page_divider_only.page_has_footnote_divider = true;
    page_divider_only.below_footnote_divider = false;
    let mut seed = lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let lines = vec![page_divider_only, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_rejects_below_divider_backward_continuation() {
    let mut below_divider = lm2_test_source_line(
        "p0:l0",
        0,
        "continuation text below the footnote divider",
        1.0,
        false,
        None,
    );
    below_divider.page_has_footnote_divider = true;
    below_divider.below_footnote_divider = true;
    let mut seed = lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let lines = vec![below_divider, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_does_not_backward_close_before_seed() {
    let mut body = lm2_test_source_line(
        "p0:l0",
        0,
        "The article returns to ordinary body prose.",
        1.0,
        false,
        None,
    );
    body.bottom = 0.62;
    body.top = 0.64;
    let mut continuation = lm2_test_source_line(
        "p0:l1",
        1,
        "continued discussion, see id.",
        0.82,
        false,
        None,
    );
    continuation.bottom = 0.32;
    continuation.top = 0.34;
    let mut seed = lm2_test_source_line(
        "p0:l2",
        2,
        "mark cases later cited the same passage",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let lines = vec![body, continuation, seed];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(
        path,
        vec![Lm2Action::Keep, Lm2Action::Keep, Lm2Action::Marginalia,]
    );
}

#[test]
fn pp_footnote_membership_does_not_backward_close_across_same_page_run() {
    let mut continuations = (0..=4)
        .map(|index| {
            let mut line = lm2_test_source_line(
                &format!("p0:l{index}"),
                index,
                &format!("continuation {index}, see id."),
                0.82,
                false,
                None,
            );
            line.bottom = 0.36 - (index as f32 * 0.01);
            line.top = 0.38 - (index as f32 * 0.01);
            line
        })
        .collect::<Vec<_>>();
    let mut seed = lm2_test_source_line("p0:l5", 5, "12. Seed region member", 0.82, false, None);
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let mut next_page_seed =
        lm2_test_source_line("p1:l0", 0, "13. Next page seed", 0.82, false, None);
    next_page_seed.page_index = 1;
    next_page_seed.bottom = 0.30;
    next_page_seed.top = 0.32;
    mark_pp_footnote(&mut next_page_seed);
    let mut lines = Vec::new();
    lines.append(&mut continuations);
    lines.push(seed);
    lines.push(next_page_seed);
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(
        path,
        vec![
            Lm2Action::Keep,
            Lm2Action::Keep,
            Lm2Action::Keep,
            Lm2Action::Keep,
            Lm2Action::Keep,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
        ]
    );
}

#[test]
fn pp_footnote_membership_forward_closes_note_like_continuation() {
    let mut seed = lm2_test_source_line(
        "p0:l0",
        0,
        "12. See United States v. Alvarez",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let mut continuation = lm2_test_source_line(
        "p0:l1",
        1,
        "continued discussion, see id.",
        0.82,
        false,
        None,
    );
    continuation.bottom = 0.28;
    continuation.top = 0.30;
    let lines = vec![seed, continuation];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_forward_closure_stops_before_body_text() {
    let mut seed = lm2_test_source_line(
        "p0:l0",
        0,
        "12. See United States v. Alvarez",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let mut body = lm2_test_source_line(
        "p0:l1",
        1,
        "The article returns to ordinary body prose.",
        1.0,
        false,
        None,
    );
    body.bottom = 0.62;
    body.top = 0.64;
    let lines = vec![seed, body];
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Keep]);
}

#[test]
fn pp_footnote_membership_forward_closure_is_bounded_and_same_page() {
    let mut seed = lm2_test_source_line(
        "p0:l0",
        0,
        "12. See United States v. Alvarez",
        0.82,
        false,
        None,
    );
    seed.bottom = 0.30;
    seed.top = 0.32;
    mark_pp_footnote(&mut seed);
    let mut continuations = (1..=17)
        .map(|index| {
            let mut line = lm2_test_source_line(
                &format!("p0:l{index}"),
                index,
                &format!("continuation {index}, see id."),
                0.82,
                false,
                None,
            );
            line.bottom = 0.30 - (index as f32 * 0.01);
            line.top = 0.32 - (index as f32 * 0.01);
            line
        })
        .collect::<Vec<_>>();
    let mut next_page = lm2_test_source_line(
        "p1:l0",
        0,
        "next-page continuation, see id.",
        0.82,
        false,
        None,
    );
    next_page.page_index = 1;
    next_page.bottom = 0.24;
    next_page.top = 0.26;
    let mut lines = vec![seed];
    lines.append(&mut continuations);
    lines.push(next_page);
    let mut path = vec![Lm2Action::Keep; lines.len()];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(
        path,
        (0..lines.len())
            .map(|index| {
                if index <= 16 {
                    Lm2Action::Marginalia
                } else {
                    Lm2Action::Keep
                }
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn pp_footnote_membership_preserves_marker_backed_start() {
    let mut line = lm2_test_source_line(
        "p0:l0",
        0,
        "12. See United States v. Alvarez",
        0.88,
        false,
        None,
    );
    line.bottom = 0.30;
    line.top = 0.32;
    line.font_ratio_doc = 0.88;
    mark_pp_footnote(&mut line);
    let lines = vec![line];
    let mut path = vec![Lm2Action::Keep];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia]);
}

#[test]
fn pp_footnote_membership_preserves_in_run_tail_continuation() {
    let first = lm2_test_source_line("p0:l0", 0, "1. Existing note start", 0.88, false, None);
    let mut tail = lm2_test_source_line(
        "p0:l1",
        1,
        "for life.”); see also 50-State Comparison",
        0.88,
        false,
        None,
    );
    tail.bottom = 0.30;
    tail.top = 0.32;
    tail.font_ratio_doc = 0.88;
    mark_pp_footnote(&mut tail);
    let lines = vec![first, tail];
    let mut path = vec![Lm2Action::Marginalia, Lm2Action::Keep];

    apply_pp_footnote_region_membership(&lines, &mut path);

    assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Marginalia]);
}

#[test]
fn synthetic_ocr_body_guard_is_scoped_to_body_like_ocr_lines() {
    let mut ocr_body = lm2_test_source_line(
        "p4:l12",
        12,
        "This ordinary bibliography entry contains enough words to be useful substantive text.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    ocr_body.synthetic_text_geometry = true;
    ocr_body.font_ratio_doc = 0.70;
    ocr_body.bottom = 0.60;
    ocr_body.top = 0.62;

    assert_eq!(
        final_lm2_action(&ocr_body, Lm2Action::HideNoise),
        Lm2Action::Keep
    );

    let mut native_body = ocr_body.clone();
    native_body.synthetic_text_geometry = false;
    assert_eq!(
        final_lm2_action(&native_body, Lm2Action::HideNoise),
        Lm2Action::HideNoise
    );

    let mut ocr_note = ocr_body;
    ocr_note.role_hint = Some(LiquidBlockRole::Marginalia);
    assert_eq!(
        final_lm2_action(&ocr_note, Lm2Action::HideNoise),
        Lm2Action::HideNoise
    );

    let mut post_context = vec![
        (native_body, Lm2Action::HideNoise),
        (ocr_note, Lm2Action::HideNoise),
    ];
    post_context[0].0.synthetic_text_geometry = true;
    post_context[0].0.role_hint = Some(LiquidBlockRole::Paragraph);
    apply_synthetic_ocr_body_preservation(&mut post_context);
    assert_eq!(post_context[0].1, Lm2Action::Keep);
    assert_eq!(post_context[1].1, Lm2Action::HideNoise);
}

#[test]
fn front_matter_abstract_recovery_repairs_mid_sentence_role_switch() {
    let texts = [
        "Most contract litigation turns on imperfect records of the parties negotiated bargains.",
        "When interpretation runs out courts must fill the remaining gap using available evidence.",
        "Scholars have assumed the surviving text provides little evidence about omitted deal terms.",
        "We tested that assumption using real agreements with negotiated provisions deliberately masked.",
        "Lay respondents recovered the hidden term about half the time above random chance.",
        "Law students and experienced lawyers performed somewhat better across the same contract problems.",
        "Large language models recovered the negotiated provisions in nearly nine cases out of ten.",
        "The surrounding agreement therefore carries more information than conventional theory has assumed.",
        "Even an incomplete contractual signal can reconstruct missing content with the right receiver.",
        "Courts can weigh those predictions as ordinary contestable evidence in an adversarial process.",
    ];
    let mut decoded = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            (
                lm2_test_source_line(
                    &format!("p0:l{index}"),
                    index,
                    text,
                    0.92,
                    false,
                    Some(LiquidBlockRole::Paragraph),
                ),
                if index < 5 {
                    Lm2Action::Keep
                } else {
                    Lm2Action::Marginalia
                },
            )
        })
        .collect::<Vec<_>>();
    decoded.push((
        lm2_test_source_line(
            "p0:l10",
            10,
            "† Professor of Law. We thank workshop participants for their helpful comments.",
            0.82,
            false,
            Some(LiquidBlockRole::Marginalia),
        ),
        Lm2Action::Marginalia,
    ));
    decoded.push((
        lm2_test_source_line(
            "p0:l11",
            11,
            "INTRODUCTION",
            1.18,
            true,
            Some(LiquidBlockRole::Heading),
        ),
        Lm2Action::Keep,
    ));

    apply_front_matter_abstract_recovery(&mut decoded);

    assert!(decoded[..10].iter().all(|(line, action)| {
        *action == Lm2Action::Keep && line.role_hint == Some(LiquidBlockRole::Abstract)
    }));
    assert_eq!(decoded[10].1, Lm2Action::Marginalia);
    assert_eq!(decoded[10].0.role_hint, Some(LiquidBlockRole::Marginalia));
    assert_eq!(decoded[11].0.role_hint, Some(LiquidBlockRole::Heading));
}

#[test]
fn front_matter_abstract_recovery_allows_short_starred_author_heading() {
    let mut decoded = vec![
        (
            lm2_test_source_line(
                "p0:l20",
                20,
                "1. Making and Modifying Contracts ................................ 1085",
                0.90,
                false,
                Some(LiquidBlockRole::Noise),
            ),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line(
                "p1:l2",
                2,
                "Danielle D’Onfro∗",
                1.10,
                true,
                Some(LiquidBlockRole::Heading),
            ),
            Lm2Action::Keep,
        ),
    ];
    decoded[1].0.page_index = 1;
    for index in 0..10 {
        let mut line = lm2_test_source_line(
            &format!("p1:l{}", index + 3),
            index + 3,
            "This abstract explains how contract restrictions reshape ownership interests in personal property across modern markets.",
            0.86,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 1;
        line.in_footnote_zone = true;
        decoded.push((line, Lm2Action::Marginalia));
    }
    let mut introduction = lm2_test_source_line(
        "p1:l13",
        13,
        "INTRODUCTION",
        1.20,
        true,
        Some(LiquidBlockRole::Heading),
    );
    introduction.page_index = 1;
    decoded.push((introduction, Lm2Action::Keep));

    apply_front_matter_abstract_recovery(&mut decoded);

    assert!(decoded[2..12].iter().all(|(line, action)| {
        *action == Lm2Action::Keep && line.role_hint == Some(LiquidBlockRole::Abstract)
    }));
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Heading));
    assert_eq!(decoded[12].0.role_hint, Some(LiquidBlockRole::Heading));
}

#[test]
fn lm2_title_recovery_prefers_article_over_issue_masthead() {
    let mut masthead = lm2_test_source_line(
        "p0:l1",
        1,
        "VOLUME 137 FEBRUARY 2024 NUMBER 4",
        1.40,
        true,
        Some(LiquidBlockRole::Title),
    );
    masthead.bold = true;

    let mut article_title = lm2_test_source_line(
        "p1:l1",
        1,
        "CONTRACT-WRAPPED PROPERTY",
        1.55,
        true,
        Some(LiquidBlockRole::Noise),
    );
    article_title.page_index = 1;
    article_title.bold = true;

    let decoded = vec![
        (masthead, Lm2Action::Keep),
        (article_title, Lm2Action::HideNoise),
    ];
    let (title, blocks, _) = build_lm2_blocks("057-contract-wrapped-property.pdf", &decoded);

    assert_eq!(title, "CONTRACT-WRAPPED PROPERTY");
    assert_eq!(blocks[0].text, "CONTRACT-WRAPPED PROPERTY");
    assert!(blocks.iter().any(|block| {
        block.text == "VOLUME 137 FEBRUARY 2024 NUMBER 4" && block.role != LiquidBlockRole::Title
    }));
    assert!(lm2_recovered_title_is_better(
        "CONTRACT-WRAPPED PROPERTY",
        "CONTRACT-WRAPPED PROPERTY Danielle D’Onfro"
    ));
    let author = lm2_test_source_line(
        "p0:l5",
        5,
        "Danielle D’Onfro",
        1.24,
        true,
        Some(LiquidBlockRole::Heading),
    );
    assert!(lm2_source_title_author_like(&author, "Danielle D’Onfro"));
}

#[test]
fn lm2_title_recovery_stops_before_starred_author_name() {
    let mut title = lm2_test_source_line(
        "p0:l1",
        1,
        "THE STRATEGIC MOOTNESS GAP",
        1.48,
        true,
        Some(LiquidBlockRole::Title),
    );
    title.bold = true;
    let author = lm2_test_source_line(
        "p0:l2",
        2,
        "Brandi M. Lupo*",
        1.0,
        true,
        Some(LiquidBlockRole::Title),
    );
    let decoded = vec![(title, Lm2Action::Keep), (author, Lm2Action::Keep)];

    let (recovered, blocks, sources) = build_lm2_blocks("columbia-law-review.pdf", &decoded);

    assert_eq!(recovered, "THE STRATEGIC MOOTNESS GAP");
    assert_eq!(blocks[0].text, "THE STRATEGIC MOOTNESS GAP");
    assert_eq!(sources[0].lines.len(), 1);
    assert!(blocks.iter().any(|block| block.text == "Brandi M. Lupo*"));
}

#[test]
fn lm2_title_recovery_stops_before_conjoined_author_names() {
    let mut title = lm2_test_source_line(
        "p0:l4",
        4,
        "HABEAS CLASS ACTIONS",
        1.48,
        true,
        Some(LiquidBlockRole::Title),
    );
    title.bold = true;
    let author = lm2_test_source_line(
        "p0:l5",
        5,
        "Lee Kovarsky & D. Theodore Rave",
        1.0,
        true,
        Some(LiquidBlockRole::Title),
    );
    let decoded = vec![(title, Lm2Action::Keep), (author, Lm2Action::Keep)];

    let (recovered, blocks, sources) = build_lm2_blocks("harvard-law-review.pdf", &decoded);

    assert_eq!(recovered, "HABEAS CLASS ACTIONS");
    assert_eq!(blocks[0].text, "HABEAS CLASS ACTIONS");
    assert_eq!(sources[0].lines.len(), 1);
}

#[test]
fn lm2_recovers_left_aligned_uppercase_title_after_article_label() {
    let label = lm2_test_source_line(
        "p0:l4",
        4,
        "ARTICLES",
        1.0,
        false,
        Some(LiquidBlockRole::Heading),
    );
    let first = lm2_test_source_line(
        "p0:l5",
        5,
        "TRADITION AND FEMINISM IN CONSTITUTIONAL",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let second = lm2_test_source_line(
        "p0:l6",
        6,
        "RIGHTS ADJUDICATION",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let author = lm2_test_source_line(
        "p0:l7",
        7,
        "Rachel Bayefsky*",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let decoded = vec![
        (label, Lm2Action::Keep),
        (first, Lm2Action::Keep),
        (second, Lm2Action::Keep),
        (author, Lm2Action::Keep),
    ];

    let (recovered, _, _) = build_lm2_blocks("virginia-law-review.pdf", &decoded);
    assert_eq!(
        recovered,
        "TRADITION AND FEMINISM IN CONSTITUTIONAL RIGHTS ADJUDICATION"
    );
}

#[test]
fn lm2_recovers_letterspaced_author_before_the_title() {
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Title,
            text: "Cross-Sovereign Policing".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: "2955 na di a banteka".to_owned(),
            label: None,
        },
    ];
    let source_ref = |line_index, text: &str, role| LiquidSourceLineRef {
        id: Some(format!("p0:l{line_index}")),
        page_index: 0,
        line_index,
        text: text.to_owned(),
        role,
        note_markers: Vec::new(),
    };
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![source_ref(
                2,
                "Cross-Sovereign Policing",
                LiquidBlockRole::Title,
            )],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                source_ref(0, "2955", LiquidBlockRole::Noise),
                source_ref(1, "na di a banteka", LiquidBlockRole::Noise),
            ],
        },
    ];

    assert_eq!(
        apply_leading_letterspaced_author_recovery(&mut blocks, &mut sources),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::AuthorInfo);
    assert_eq!(blocks[1].text, "Nadia Banteka");
    assert_eq!(sources[1].lines.len(), 1);
}

#[test]
fn lm2_title_join_removes_only_a_physical_line_end_hyphen_space() {
    assert_eq!(
        join_lm2_title_parts(&[
            "Court-Stripping, Court-Packing, and Court-".to_owned(),
            "Defying: Judicial Reform and Constitutional Conflict".to_owned(),
        ]),
        "Court-Stripping, Court-Packing, and Court-Defying: Judicial Reform and Constitutional Conflict"
    );
    assert_eq!(
        join_lm2_title_parts(&["Law - Politics".to_owned()]),
        "Law - Politics"
    );
}

#[test]
fn lm2_recovers_title_from_hidden_leading_source_lines() {
    let decoded = vec![
        (
            lm2_test_source_line(
                "p0:l0",
                0,
                "SPERBER IN PRINTER PREP (DO NOT DELETE) 3/17/2026 10:46 AM",
                0.61,
                false,
                Some(LiquidBlockRole::Noise),
            ),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line(
                "p0:l1",
                1,
                "TESTING DOBBS'S DEMOCRACY PREMISE: CAN",
                1.36,
                true,
                None,
            ),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line(
                "p0:l2",
                2,
                "STATE CONSTITUTIONS BE AMENDED TO",
                1.40,
                true,
                None,
            ),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line(
                "p0:l3",
                3,
                "REFLECT POPULAR OPINION ON ABORTION?",
                1.40,
                true,
                Some(LiquidBlockRole::Paragraph),
            ),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l4", 4, "ISABEL SPERBER†", 0.94, true, None),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line("p0:l5", 5, "ABSTRACT", 0.92, true, None),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line(
                "p0:l6",
                6,
                "When the Supreme Court eliminated a federal constitutional right to abortion",
                1.0,
                false,
                Some(LiquidBlockRole::Paragraph),
            ),
            Lm2Action::Keep,
        ),
    ];

    let (title, blocks, sources) = build_lm2_blocks("duke_law_journal.pdf", &decoded);

    assert_eq!(
        title,
        "TESTING DOBBS'S DEMOCRACY PREMISE: CAN STATE CONSTITUTIONS BE AMENDED TO REFLECT POPULAR OPINION ON ABORTION?"
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Title);
    assert_eq!(blocks[0].text, title);
    assert!(blocks.iter().all(|block| block.text != "ISABEL SPERBER†"));
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Title)
            .count(),
        1
    );
    assert_eq!(sources[0].lines.len(), 3);
}

#[test]
fn lm2_recovers_page_one_title_after_running_header() {
    let mut running_header = lm2_test_source_line(
        "p1:l0",
        0,
        "2018] Review: Civil Justice Reconsidered 509",
        0.96,
        true,
        Some(LiquidBlockRole::Noise),
    );
    running_header.page_index = 1;

    let mut title_one = lm2_test_source_line(
        "p1:l1",
        1,
        "Book Review: Civil Justice",
        1.63,
        true,
        Some(LiquidBlockRole::Noise),
    );
    title_one.page_index = 1;
    title_one.bold = true;

    let mut title_two = lm2_test_source_line(
        "p1:l2",
        2,
        "Reconsidered: Toward a Less Costly,",
        1.63,
        true,
        Some(LiquidBlockRole::Paragraph),
    );
    title_two.page_index = 1;
    title_two.bold = true;

    let mut title_three = lm2_test_source_line(
        "p1:l3",
        3,
        "More Accessible Litigation System",
        1.63,
        true,
        None,
    );
    title_three.page_index = 1;
    title_three.bold = true;

    let decoded = vec![
        (running_header, Lm2Action::HideNoise),
        (title_one, Lm2Action::HideNoise),
        (title_two, Lm2Action::Keep),
        (title_three, Lm2Action::Keep),
    ];

    let (title, blocks, _) = build_lm2_blocks("book-review.pdf", &decoded);

    assert_eq!(
        title,
        "Book Review: Civil Justice Reconsidered: Toward a Less Costly, More Accessible Litigation System"
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Title);
    assert_eq!(blocks[0].text, title);
    assert!(blocks.iter().any(|block| block.text
        == "2018] Review: Civil Justice Reconsidered 509"
        && block.role == LiquidBlockRole::Noise));
}

#[test]
fn lm2_recovers_deduplicated_synthetic_ocr_title() {
    let mut title_one = lm2_test_source_line(
        "p0:l0",
        0,
        "SLICING DEFAMATION BY CONTRACT",
        1.0,
        false,
        Some(LiquidBlockRole::Heading),
    );
    title_one.synthetic_text_geometry = true;
    let mut title_duplicate = title_one.clone();
    title_duplicate.id = "p0:l1".to_owned();
    title_duplicate.line_index = 1;
    let mut author = lm2_test_source_line("p0:l2", 2, "Yonathan A. Arbel", 1.0, false, None);
    author.synthetic_text_geometry = true;

    let decoded = vec![
        (title_one, Lm2Action::HideNoise),
        (title_duplicate, Lm2Action::HideNoise),
        (author, Lm2Action::HideNoise),
    ];
    let (title, blocks, _) = build_lm2_blocks("Slicing SSRN.pdf", &decoded);

    assert_eq!(title, "SLICING DEFAMATION BY CONTRACT");
    assert_eq!(blocks[0].role, LiquidBlockRole::Title);
    assert_eq!(blocks[0].text, "SLICING DEFAMATION BY CONTRACT");
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.text == "SLICING DEFAMATION BY CONTRACT")
            .count(),
        1
    );
}

#[test]
fn lm2_source_title_recovery_collapses_visible_multiline_title() {
    let decoded = vec![
        (
            lm2_test_source_line(
                "p0:l1",
                1,
                "TESTING DOBBS'S DEMOCRACY PREMISE: CAN",
                1.36,
                true,
                None,
            ),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line(
                "p0:l2",
                2,
                "STATE CONSTITUTIONS BE AMENDED TO",
                1.40,
                true,
                None,
            ),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line(
                "p0:l3",
                3,
                "REFLECT POPULAR OPINION ON ABORTION?",
                1.40,
                true,
                Some(LiquidBlockRole::Paragraph),
            ),
            Lm2Action::Keep,
        ),
    ];

    let (_, blocks, _) = build_lm2_blocks("", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Title);
    assert_eq!(
        blocks[0].text,
        "TESTING DOBBS'S DEMOCRACY PREMISE: CAN STATE CONSTITUTIONS BE AMENDED TO REFLECT POPULAR OPINION ON ABORTION?"
    );
}

#[test]
fn pymupdf_grouping_overrides_paragraph_boundary_for_assigned_lines() {
    let decoded = vec![
        (
            lm2_test_source_line("p0:l0", 0, "First paragraph line.", 1.0, false, None),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l1", 1, "Second line in same box.", 1.0, false, None),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l2", 2, "Next box starts here.", 1.0, false, None),
            Lm2Action::Keep,
        ),
    ];
    let grouping = Lm2PymupdfGroupingResponse {
        mode: Some("pymupdf".to_owned()),
        warnings: Vec::new(),
        blocks: vec![
            Lm2PymupdfGroupingBlock {
                block_index: Some(0),
                page_index: Some(0),
                source: Some("pymupdf".to_owned()),
                source_line_ids: vec!["p0:l0".to_owned(), "p0:l1".to_owned()],
            },
            Lm2PymupdfGroupingBlock {
                block_index: Some(1),
                page_index: Some(0),
                source: Some("pymupdf".to_owned()),
                source_line_ids: vec!["p0:l2".to_owned()],
            },
        ],
    };

    let (_, blocks, sources) = build_lm2_blocks_with_grouping("", &decoded, Some(&grouping), &[]);
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        "First paragraph line. Second line in same box."
    );
    assert_eq!(blocks[1].text, "Next box starts here.");
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .filter_map(|line| line.id.clone())
            .collect::<Vec<_>>(),
        vec!["p0:l0".to_owned(), "p0:l1".to_owned()]
    );
    assert_eq!(
        sources[1]
            .lines
            .iter()
            .filter_map(|line| line.id.clone())
            .collect::<Vec<_>>(),
        vec!["p0:l2".to_owned()]
    );
}

#[test]
fn paragraph_continues_across_source_page_without_indent() {
    let previous = lm2_test_source_line(
        "p0:l9",
        9,
        "The rule continues across the source page.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut next = lm2_test_source_line(
        "p1:l0",
        0,
        "Its application remains contested.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next.page_index = 1;
    assert!(!paragraph_boundary(&previous, &next));
}

#[test]
fn clear_indent_can_start_approximate_paragraph_across_page() {
    let previous = lm2_test_source_line(
        "p0:l9",
        9,
        "The first paragraph ends here.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut next = lm2_test_source_line(
        "p1:l0",
        0,
        "A new paragraph begins here.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next.page_index = 1;
    next.left = previous.left + 0.04;
    assert!(paragraph_boundary(&previous, &next));
}

#[test]
fn d1_runtime_zerospend_overlay_recovers_small_font_citation_keep_line() {
    let mut line = lm2_test_source_line(
        "p0:l18",
        18,
        "45. Koons v. Platkin, 673 F. Supp. 3d 515, 620 (D.N.J. 2023).",
        0.86,
        false,
        None,
    );
    line.font_ratio_doc = 0.86;
    line.font_ratio_page_ref = 0.86;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_d1_runtime_zerospend_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_zerospend_overlay_preserves_uncued_body_keep_line() {
    let mut line = lm2_test_source_line(
        "p0:l18",
        18,
        "Freedom]. In Presuming Trustworthiness, we investigated the Justices abandonment.",
        0.86,
        false,
        None,
    );
    line.font_ratio_doc = 0.86;
    line.font_ratio_page_ref = 0.86;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_d1_runtime_zerospend_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_zerospend_overlay_requires_lower_page_region() {
    let mut line = lm2_test_source_line(
        "p0:l4",
        4,
        "See Brown v. Board of Education, 347 U.S. 483 (1954).",
        0.86,
        false,
        None,
    );
    line.font_ratio_doc = 0.86;
    line.font_ratio_page_ref = 0.86;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_d1_runtime_zerospend_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn d1_runtime_continuation_overlay_recovers_after_marginalia_anchor() {
    let anchor = lm2_test_source_line(
        "p0:l12",
        12,
        "1. Detention FY 2025 YTD, U.S. Immigr. & Customs Enf't",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut continuation = lm2_test_source_line(
        "p0:l13",
        13,
        "Immigr. & Customs Enf't (2025), https://www.ice.gov/doclib/detention/FY25.xlsx",
        0.86,
        false,
        None,
    );
    continuation.bottom = 0.30;
    continuation.font_ratio_doc = 0.86;
    let mut decoded = vec![
        (anchor, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
    ];

    apply_d1_runtime_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_continuation_overlay_stops_before_uncued_body_prose() {
    let anchor = lm2_test_source_line(
        "p0:l12",
        12,
        "1. Detention FY 2025 YTD, U.S. Immigr. & Customs Enf't",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut body = lm2_test_source_line(
        "p0:l13",
        13,
        "The next section turns from detention statistics to doctrine.",
        0.86,
        false,
        None,
    );
    body.bottom = 0.30;
    body.font_ratio_doc = 0.86;
    let mut decoded = vec![(anchor, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

    apply_d1_runtime_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_immediate_continuation_overlay_recovers_sandwiched_small_font_line() {
    let previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut continuation = lm2_test_source_line(
        "p0:l13",
        13,
        "explaining the same doctrine in later agency guidance.",
        0.86,
        false,
        None,
    );
    continuation.font_ratio_doc = 0.86;
    let next = lm2_test_source_line(
        "p0:l14",
        14,
        "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
        (next, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_immediate_continuation_overlay_requires_next_marginalia() {
    let previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut candidate = lm2_test_source_line(
        "p0:l13",
        13,
        "The next section turns from detention statistics to doctrine.",
        0.86,
        false,
        None,
    );
    candidate.font_ratio_doc = 0.86;
    let next = lm2_test_source_line("p0:l14", 14, "II. Doctrine", 1.0, false, None);
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (candidate, Lm2Action::Keep),
        (next, Lm2Action::Keep),
    ];

    apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_immediate_continuation_overlay_does_not_cross_pages() {
    let mut previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    previous.page_index = 0;
    let mut candidate = lm2_test_source_line(
        "p1:l0",
        0,
        "explaining the same doctrine in later agency guidance.",
        0.86,
        false,
        None,
    );
    candidate.page_index = 1;
    candidate.font_ratio_doc = 0.86;
    let mut next = lm2_test_source_line(
        "p1:l1",
        1,
        "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    next.page_index = 1;
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (candidate, Lm2Action::Keep),
        (next, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_sandwiched_continuation_overlay_accepts_neighbor_with_one_line_gap() {
    let previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let body_gap = lm2_test_source_line("p0:l13", 13, "intervening body line", 1.0, false, None);
    let mut continuation = lm2_test_source_line(
        "p0:l14",
        14,
        "explaining the same doctrine in later agency guidance.",
        0.86,
        false,
        None,
    );
    continuation.font_ratio_doc = 0.86;
    let next = lm2_test_source_line(
        "p0:l15",
        15,
        "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (body_gap, Lm2Action::Keep),
        (continuation, Lm2Action::Keep),
        (next, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);

    assert_eq!(decoded[2].1, Lm2Action::Marginalia);
    assert_eq!(decoded[2].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_sandwiched_continuation_overlay_does_not_cross_pages() {
    let mut previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    previous.page_index = 0;
    let mut candidate = lm2_test_source_line(
        "p1:l0",
        0,
        "explaining the same doctrine in later agency guidance.",
        0.86,
        false,
        None,
    );
    candidate.page_index = 1;
    candidate.font_ratio_doc = 0.86;
    let mut next = lm2_test_source_line(
        "p1:l1",
        1,
        "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    next.page_index = 1;
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (candidate, Lm2Action::Keep),
        (next, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_wide_sandwich_overlay_accepts_font095_sandwich() {
    let previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut continuation = lm2_test_source_line(
        "p0:l13",
        13,
        "explaining the same doctrine in later agency guidance.",
        0.95,
        false,
        None,
    );
    continuation.font_ratio_doc = 0.95;
    let next = lm2_test_source_line(
        "p0:l14",
        14,
        "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
        (next, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_wide_sandwich_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_wide_sandwich_overlay_requires_next_marginalia() {
    let previous = lm2_test_source_line(
        "p0:l12",
        12,
        "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut candidate = lm2_test_source_line(
        "p0:l13",
        13,
        "The next section turns from detention statistics to doctrine.",
        0.95,
        false,
        None,
    );
    candidate.font_ratio_doc = 0.95;
    let next = lm2_test_source_line("p0:l14", 14, "II. Doctrine", 1.0, false, None);
    let mut decoded = vec![
        (previous, Lm2Action::Marginalia),
        (candidate, Lm2Action::Keep),
        (next, Lm2Action::Keep),
    ];

    apply_d1_runtime_wide_sandwich_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_post_wide_cue_overlay_accepts_forward_cued_line() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "12. See the same source for this proposition.",
        0.88,
        false,
        None,
    );
    let next_keep = lm2_test_source_line(
        "p0:l21",
        21,
        "intermediate carried body line",
        1.0,
        false,
        None,
    );
    let next_marginalia = lm2_test_source_line(
        "p0:l22",
        22,
        "continuing the footnote discussion.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let next_marginalia_two = lm2_test_source_line(
        "p0:l23",
        23,
        "more authority for the same point.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (next_keep, Lm2Action::Keep),
        (next_marginalia, Lm2Action::Marginalia),
        (next_marginalia_two, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_post_wide_cue_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_post_wide_cue_overlay_rejects_uncued_body_line() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "This Part next explains why the doctrine developed slowly.",
        0.88,
        false,
        None,
    );
    let next_marginalia = lm2_test_source_line(
        "p0:l21",
        21,
        "continuing the footnote discussion.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let next_marginalia_two = lm2_test_source_line(
        "p0:l22",
        22,
        "more authority for the same point.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (next_marginalia, Lm2Action::Marginalia),
        (next_marginalia_two, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_post_wide_cue_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_postcue_citation_next1_overlay_accepts_citation_before_marginalia() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "See https://perma.cc/ABCD-EFGH for the archived source.",
        0.95,
        false,
        None,
    );
    let next_marginalia = lm2_test_source_line(
        "p0:l21",
        21,
        "continuing the same footnote.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (next_marginalia, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_postcue_citation_next1_overlay_requires_next_marginalia() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "See https://perma.cc/ABCD-EFGH for the archived source.",
        0.95,
        false,
        None,
    );
    let next_keep = lm2_test_source_line(
        "p0:l21",
        21,
        "The Article then returns to ordinary body prose.",
        1.0,
        false,
        None,
    );
    let mut decoded = vec![(candidate, Lm2Action::Keep), (next_keep, Lm2Action::Keep)];

    apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_postcue_citation_next1_overlay_rejects_bare_ibid_index_line() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "child, and subsequently, were inadmissible to prove it illegitimate. Ibid.",
        0.87,
        false,
        None,
    );
    let next_marginalia = lm2_test_source_line(
        "p0:l21",
        21,
        "1. In an inheritance case, where the claimant was begotten before",
        0.87,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (next_marginalia, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_near8_cue_overlay_accepts_cued_line_near_four_marginalia() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "Analysis, 399 THE LANCET 629, 639 (2022), describing the global burden of bacterial resistance.",
        0.84,
        false,
        None,
    );
    let m1 = lm2_test_source_line("p0:l21", 21, "continuing footnote one", 0.84, false, None);
    let m2 = lm2_test_source_line("p0:l22", 22, "continuing footnote two", 0.84, false, None);
    let m3 = lm2_test_source_line("p0:l23", 23, "continuing footnote three", 0.84, false, None);
    let m4 = lm2_test_source_line("p0:l24", 24, "continuing footnote four", 0.84, false, None);
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (m1, Lm2Action::Marginalia),
        (m2, Lm2Action::Marginalia),
        (m3, Lm2Action::Marginalia),
        (m4, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_near8_cue_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_near8_cue_overlay_requires_four_nearby_marginalia() {
    let candidate = lm2_test_source_line(
        "p0:l20",
        20,
        "Analysis, 399 THE LANCET 629, 639 (2022), describing the global burden of bacterial resistance.",
        0.84,
        false,
        None,
    );
    let m1 = lm2_test_source_line("p0:l21", 21, "continuing footnote one", 0.84, false, None);
    let m2 = lm2_test_source_line("p0:l22", 22, "continuing footnote two", 0.84, false, None);
    let m3 = lm2_test_source_line("p0:l23", 23, "continuing footnote three", 0.84, false, None);
    let mut decoded = vec![
        (candidate, Lm2Action::Keep),
        (m1, Lm2Action::Marginalia),
        (m2, Lm2Action::Marginalia),
        (m3, Lm2Action::Marginalia),
    ];

    apply_d1_runtime_near8_cue_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn axis_numeric_token_handles_multibyte_currency_prefix() {
    assert!(lm2_axis_numeric_token("£1,200"));
    assert!(lm2_axis_numeric_token("€99.50"));
    assert!(lm2_axis_numeric_token("($1,200)"));
}

#[test]
fn table_figure_router_env_opt_out_accepts_common_falsey_values() {
    for value in ["0", "false", "FALSE", " no ", "off"] {
        assert!(falsey_env_value(value));
    }
    for value in ["", "1", "true", "yes", "default"] {
        assert!(!falsey_env_value(value));
    }
}

#[test]
fn page_object_tuned_preset_alias_is_explicit() {
    assert!(lm2_runtime_preset_is_page_object_tuned(
        LM2_V25_D1_PAGE_OBJECT_TUNED_PRESET
    ));
    assert!(lm2_runtime_preset_is_page_object_tuned(
        "V25-D1-SANDWICHED-NOTE-START-WIDE-SANDWICH-POSTCUE-CITATION-NEXT1-NEAR8CUE-WIDE-DIVIDER-GUARD-PAGE-OBJECT-TUNED"
    ));
    assert!(!lm2_runtime_preset_is_page_object_tuned(
        "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard"
    ));
}

#[test]
fn table_figure_router_routes_numeric_grid_row_to_hide_noise() {
    let mut candidate = lm2_test_source_line(
        "p0:l40",
        40,
        "Nassau 21 97 21.6% 31 124 25.0% 31 121 25.6%",
        1.0,
        false,
        None,
    );
    candidate.page_width = 1.0;
    candidate.left = 0.1;
    candidate.right = 0.45; // width_norm 0.35 < 0.6
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_table_figure_router_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    // Tagged Table (not Noise) so a future display-tables toggle can resurface it.
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Table));
}

#[test]
fn lm2_assembler_preserves_router_table_lines_as_table_blocks() {
    let mut candidate = lm2_test_source_line(
        "p0:l40",
        40,
        "Nassau 21 97 21.6% 31 124 25.0% 31 121 25.6%",
        1.0,
        false,
        Some(LiquidBlockRole::Table),
    );
    candidate.page_width = 1.0;
    candidate.left = 0.1;
    candidate.right = 0.45;
    let decoded = vec![(candidate, Lm2Action::HideNoise)];

    let (_title, blocks, sources) = build_lm2_blocks("Fallback", &decoded);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Table);
    assert_eq!(blocks[0].label.as_deref(), Some("Table/Figure"));
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].lines[0].role, LiquidBlockRole::Table);
}

#[test]
fn lm2_assembler_drops_router_header_furniture() {
    let mut candidate = lm2_test_source_line(
        "p3:l1",
        1,
        "68 UCLA L. REV. DISC. (LAW MEETS WORLD) 22 (2020)",
        1.0,
        false,
        Some(LiquidBlockRole::Header),
    );
    candidate.doc_repeated_text_count = 5;
    let decoded = vec![(candidate, Lm2Action::HideNoise)];

    let (_title, blocks, sources) = build_lm2_blocks("Fallback", &decoded);

    // Furniture is routed to Noise, not deleted: the generator decides
    // what to drop, where the decision can be seen and measured.
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
    assert_eq!(sources.len(), 1);
}

#[test]
fn lm2_assembler_does_not_merge_noise_across_page_boundaries() {
    let mut footer = lm2_test_source_line(
        "p0:l20",
        20,
        "Electronic Paper Collection: https://example.test",
        1.0,
        false,
        None,
    );
    footer.page_index = 0;
    let mut next_page_title =
        lm2_test_source_line("p1:l0", 0, "Civil Justice Reconsidered", 1.2, true, None);
    next_page_title.page_index = 1;
    let decoded = vec![
        (footer, Lm2Action::HideNoise),
        (next_page_title, Lm2Action::HideNoise),
    ];

    let (_title, blocks, sources) = build_lm2_blocks("Fallback", &decoded);

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
    assert_eq!(blocks[1].role, LiquidBlockRole::Noise);
    assert_eq!(sources[0].lines[0].page_index, 0);
    assert_eq!(sources[1].lines[0].page_index, 1);
}

#[test]
fn table_figure_router_leaves_numeric_citation_prose_as_keep() {
    let mut candidate = lm2_test_source_line(
        "p0:l41",
        41,
        "Law, 7 J. Legal Stud. 393 (1979).",
        1.0,
        false,
        None,
    );
    candidate.page_width = 1.0;
    candidate.left = 0.1;
    candidate.right = 0.45;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_table_figure_router_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn table_figure_router_routes_repeated_running_header_to_hide_noise() {
    let mut candidate = lm2_test_source_line(
        "p3:l1",
        1,
        "68 UCLA L. REV. DISC. (LAW MEETS WORLD) 22 (2020)",
        1.0,
        false,
        None,
    );
    candidate.doc_repeated_text_count = 5;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_table_figure_router_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    // Running-header furniture is tagged Header (not resurfaced by the toggle).
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Header));
}

#[test]
fn table_figure_router_keeps_repeated_body_sentence() {
    let mut candidate = lm2_test_source_line(
        "p3:l2",
        2,
        "The court held that the statute was unconstitutional on its face.",
        1.0,
        false,
        None,
    );
    candidate.doc_repeated_text_count = 4;
    candidate.page_width = 1.0;
    candidate.left = 0.1;
    candidate.right = 0.9;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_table_figure_router_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn page_object_tuned_overlay_hides_ruled_keep_and_marginalia_rows() {
    let mut keep = lm2_test_source_line("p0:l1", 1, "County 2020 2021 2022", 1.0, false, None);
    keep.page_width = 600.0;
    keep.page_height = 800.0;
    keep.left = 80.0;
    keep.right = 520.0;
    keep.bottom = 500.0;
    keep.top = 512.0;
    keep.page_object_ruled_row_membership = true;
    let mut marginalia = keep.clone();
    marginalia.id = "p0:l2".to_owned();
    marginalia.line_index = 2;
    let mut decoded = vec![(keep, Lm2Action::Keep), (marginalia, Lm2Action::Marginalia)];

    apply_page_object_tuned_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Table));
    assert_eq!(decoded[1].1, Lm2Action::HideNoise);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Table));
}

#[test]
fn page_object_tuned_overlay_preserves_legal_ruled_keep_rows() {
    let mut row = lm2_test_source_line(
        "p0:l10",
        10,
        "Montana Court Channeling Mont. Code Ann. § 46-21-",
        1.0,
        false,
        None,
    );
    row.page_width = 600.0;
    row.page_height = 800.0;
    row.left = 120.0;
    row.right = 500.0;
    row.bottom = 500.0;
    row.top = 512.0;
    row.page_object_ruled_row_membership = true;
    let mut decoded = vec![(row, Lm2Action::Keep)];

    apply_page_object_tuned_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn page_object_tuned_overlay_preserves_prose_like_ruled_keep_rows() {
    let mut row = lm2_test_source_line(
        "p0:l11",
        11,
        "The court concluded that the statute remained available after conviction.",
        1.0,
        false,
        None,
    );
    row.page_width = 600.0;
    row.page_height = 800.0;
    row.left = 80.0;
    row.right = 520.0;
    row.bottom = 500.0;
    row.top = 512.0;
    row.page_object_ruled_row_membership = true;
    let mut decoded = vec![(row, Lm2Action::Keep)];

    apply_page_object_tuned_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn page_object_tuned_overlay_rescues_body_like_nonedge_rows() {
    let mut body = lm2_test_source_line(
        "p0:l5",
        5,
        "The court held that the statutory claim remains available to plaintiffs.",
        0.90,
        true,
        None,
    );
    body.page_width = 600.0;
    body.page_height = 800.0;
    body.left = 70.0;
    body.right = 520.0;
    body.bottom = 420.0;
    body.top = 432.0;
    let mut decoded = vec![(body, Lm2Action::HideNoise)];

    apply_page_object_tuned_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Paragraph));
}

#[test]
fn page_object_tuned_overlay_does_not_rescue_footnote_zone_rows() {
    let mut note = lm2_test_source_line(
        "p0:l30",
        30,
        "The court held that the statutory claim remains available to plaintiffs.",
        0.90,
        true,
        None,
    );
    note.page_width = 600.0;
    note.page_height = 800.0;
    note.left = 70.0;
    note.right = 520.0;
    note.bottom = 420.0;
    note.top = 432.0;
    note.in_footnote_zone = true;
    let mut decoded = vec![(note, Lm2Action::Marginalia)];

    apply_page_object_tuned_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_accepts_small_lower_short_line_below_divider() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "federal courts applying the same equitable doctrine.",
        0.88,
        false,
        None,
    );
    candidate.page_index = 2;
    candidate.below_footnote_divider = true;
    candidate.font_ratio_page_ref = 0.88;
    candidate.top = 400.0;
    candidate.bottom = 388.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_requires_below_divider() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "federal courts applying the same equitable doctrine.",
        0.88,
        false,
        None,
    );
    candidate.font_ratio_page_ref = 0.88;
    candidate.top = 400.0;
    candidate.bottom = 388.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_rejects_first_page_front_matter() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "Copyright 2023 Laura Portuondo.",
        0.88,
        false,
        None,
    );
    candidate.page_index = 0;
    candidate.below_footnote_divider = true;
    candidate.font_ratio_page_ref = 0.88;
    candidate.top = 400.0;
    candidate.bottom = 388.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_rejects_large_font_line() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "federal courts applying the same equitable doctrine.",
        0.98,
        false,
        None,
    );
    candidate.below_footnote_divider = true;
    candidate.font_ratio_page_ref = 0.98;
    candidate.top = 400.0;
    candidate.bottom = 388.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_rejects_upper_page_line() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "federal courts applying the same equitable doctrine.",
        0.88,
        false,
        None,
    );
    candidate.below_footnote_divider = true;
    candidate.font_ratio_page_ref = 0.88;
    candidate.top = 620.0;
    candidate.bottom = 608.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn d1_runtime_wide_divider_guard_overlay_rejects_table_stat_line() {
    let mut candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "Table 1 2020 2021 2022 45% 51%",
        0.88,
        false,
        None,
    );
    candidate.below_footnote_divider = true;
    candidate.font_ratio_page_ref = 0.88;
    candidate.top = 400.0;
    candidate.bottom = 388.0;
    candidate.page_height = 1000.0;
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn d1_runtime_footer_artifact_overlay_hides_indd_footer() {
    let candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "jobname: article-22-3-1.indd PDFOutput Page 1",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![(candidate, Lm2Action::Marginalia)];

    apply_d1_runtime_footer_artifact_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn d1_runtime_footer_artifact_overlay_hides_repository_footer() {
    let candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "This Article is brought to you for free and open access by the Law Archive of Scholarship.",
        0.88,
        false,
        None,
    );
    let mut decoded = vec![(candidate, Lm2Action::Keep)];

    apply_d1_runtime_footer_artifact_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn d1_runtime_footer_artifact_overlay_rejects_author_contact_lines() {
    let candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "For more information, please contact repository@example.edu or phone: 555-0100.",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    let mut decoded = vec![(candidate, Lm2Action::Marginalia)];

    apply_d1_runtime_footer_artifact_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn hidden_short_numbered_note_head_is_restored_from_physical_note_zone() {
    let mut hidden = lm2_test_source_line(
        "p4:l53",
        53,
        "113. Id.",
        0.81,
        false,
        Some(LiquidBlockRole::Noise),
    );
    hidden.font_ratio_page_ref = 0.81;
    hidden.in_footnote_zone = true;
    hidden.page_has_footnote_divider = true;
    hidden.below_footnote_divider = true;
    // `Id.` definitions repeat at the same edge position across pages;
    // explicit divider geometry must outrank that furniture prior.
    hidden.doc_repeated_edge_text = true;
    let mut citation = hidden.clone();
    citation.id = "p4:l54".to_owned();
    citation.line_index = 54;
    citation.text = "106 VA. L. REV. 611".to_owned();
    let mut decoded = vec![
        (hidden, Lm2Action::HideNoise),
        (citation, Lm2Action::HideNoise),
    ];

    assert_eq!(apply_hidden_numbered_note_head_recovery(&mut decoded), 1);
    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.doc_note_marker, 113);
    assert_eq!(decoded[1].1, Lm2Action::HideNoise);
}

#[test]
fn hidden_short_numbered_note_block_is_restored_with_source_marker() {
    let mut line = lm2_test_source_line(
        "p18:l47",
        47,
        "113. Id.",
        0.81,
        false,
        Some(LiquidBlockRole::Noise),
    );
    line.page_has_footnote_divider = true;
    line.below_footnote_divider = true;
    line.font_ratio_page_ref = 0.81;
    line.doc_repeated_edge_text = true;
    let decoded = vec![(line.clone(), Lm2Action::HideNoise)];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Noise,
        text: line.text.clone(),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![line_ref(&line, LiquidBlockRole::Noise)],
    }];

    assert_eq!(
        apply_hidden_numbered_note_block_recovery(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Marginalia);
    assert_eq!(sources[0].lines[0].note_markers, vec![113]);
}

#[test]
fn hidden_note_block_trusts_matching_marginalia_provenance_marker() {
    let mut line = lm2_test_source_line(
        "p46:l39",
        39,
        "380 See Ortiz v. Fibreboard Corp., 527 U.S. 815, 842 (1999).",
        1.0,
        false,
        Some(LiquidBlockRole::Noise),
    );
    line.in_footnote_zone = false;
    line.below_footnote_divider = false;
    let decoded = vec![(line.clone(), Lm2Action::HideNoise)];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Noise,
        text: line.text.clone(),
        label: None,
    }];
    let mut source_line = line_ref(&line, LiquidBlockRole::Marginalia);
    source_line.note_markers = vec![380];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![source_line],
    }];

    assert_eq!(
        apply_hidden_numbered_note_block_recovery(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Marginalia);
    assert_eq!(sources[0].lines[0].note_markers, vec![380]);
}

#[test]
fn tiny_boundary_paragraph_note_head_is_restored_before_sequential_note() {
    let mut body = lm2_test_source_line(
        "p78:l33",
        33,
        "and other modalities in depth. As part of this examination, I intend to",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 78;
    let mut note_head = lm2_test_source_line(
        "p78:l34",
        34,
        "341 The term modalities is most associated with the work of Bobbitt, who defined a",
        0.764,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    note_head.page_index = 78;
    note_head.font_ratio_page_ref = 0.764;
    note_head.font_ratio_doc = 0.764;
    note_head.segment_block_id = 3;
    note_head.segment_block_line_index = 0;
    note_head.segment_block_line_count = 2;
    note_head.segment_block_shape = "body".to_owned();
    let mut note_head_tail = note_head.clone();
    note_head_tail.id = "p78:l35".to_owned();
    note_head_tail.line_index = 35;
    note_head_tail.text =
        "modality in a technical sense as the way in which we characterize expression as"
            .to_owned();
    note_head_tail.segment_block_line_index = 1;
    let mut continuation = lm2_test_source_line(
        "p78:l36",
        36,
        "true. Bobbitt, supra note 118, at 11.",
        0.789,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    continuation.page_index = 78;
    continuation.font_ratio_page_ref = 0.789;
    continuation.font_ratio_doc = 0.789;
    continuation.in_footnote_zone = true;
    continuation.segment_block_id = 4;
    continuation.segment_block_line_index = 0;
    continuation.segment_block_line_count = 2;
    continuation.segment_block_shape = "footnote".to_owned();
    continuation.segment_block_footnote_like = true;
    let mut next_note = continuation.clone();
    next_note.id = "p78:l37".to_owned();
    next_note.line_index = 37;
    next_note.text = "342 Griffin, supra note 23, at 1753.".to_owned();
    next_note.segment_block_line_index = 1;
    let mut next_body = lm2_test_source_line(
        "p79:l2",
        2,
        "argue that other modalities can help the interpreter.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    next_body.page_index = 79;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: body.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!(
                "{CALLOUT_START}341{CALLOUT_END} The term modalities is most associated with the work of Bobbitt, who defined a modality in a technical sense as the way in which we characterize expression as"
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!(
                "true. Bobbitt, supra note 118, at 11. {CALLOUT_START}342{CALLOUT_END} Griffin, supra note 23, at 1753."
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: next_body.text.clone(),
            label: None,
        },
    ];
    let mut next_note_ref = line_ref(&next_note, LiquidBlockRole::Marginalia);
    next_note_ref.note_markers = vec![342];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&note_head, LiquidBlockRole::Paragraph),
                line_ref(&note_head_tail, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![
                line_ref(&continuation, LiquidBlockRole::Marginalia),
                next_note_ref,
            ],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&next_body, LiquidBlockRole::Paragraph)],
        },
    ];
    let decoded = vec![
        (body, Lm2Action::Keep),
        (note_head, Lm2Action::Keep),
        (note_head_tail, Lm2Action::Keep),
        (continuation, Lm2Action::Marginalia),
        (next_note, Lm2Action::Marginalia),
        (next_body, Lm2Action::Keep),
    ];

    assert_eq!(
        apply_hidden_numbered_note_block_recovery(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(blocks[0].text.ends_with("I intend to"));
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(sources[1].lines[0].note_markers, vec![341]);
    assert!(sources[1].lines[1].note_markers.is_empty());
    assert!(
        sources[1]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Marginalia)
    );
    assert_eq!(sources[2].lines[1].note_markers, vec![342]);
    assert_eq!(blocks[3].role, LiquidBlockRole::Paragraph);
}

#[test]
fn tiny_numbered_body_paragraph_requires_markerless_prefix_and_next_marker() {
    let mut candidate = lm2_test_source_line(
        "p8:l20",
        20,
        "341 A numbered body paragraph that must remain body",
        0.78,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    candidate.page_index = 8;
    candidate.font_ratio_page_ref = 0.78;
    candidate.font_ratio_doc = 0.78;
    candidate.segment_block_id = 3;
    let mut following = lm2_test_source_line(
        "p8:l21",
        21,
        "343 A nonsequential footnote definition.",
        0.78,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    following.page_index = 8;
    following.font_ratio_page_ref = 0.78;
    following.font_ratio_doc = 0.78;
    following.in_footnote_zone = true;
    following.segment_block_shape = "footnote".to_owned();
    following.segment_block_footnote_like = true;
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: format!(
                "{CALLOUT_START}341{CALLOUT_END} A numbered body paragraph that must remain body"
            ),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: format!("{CALLOUT_START}343{CALLOUT_END} A nonsequential footnote definition."),
            label: None,
        },
    ];
    let mut following_ref = line_ref(&following, LiquidBlockRole::Marginalia);
    following_ref.note_markers = vec![343];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&candidate, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![following_ref],
        },
    ];
    let decoded = vec![
        (candidate, Lm2Action::Keep),
        (following, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_hidden_numbered_note_block_recovery(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert!(sources[0].lines[0].note_markers.is_empty());
}

#[test]
fn body_sized_marginalia_above_first_note_head_returns_to_body() {
    let mut body = lm2_test_source_line(
        "p4:l42",
        42,
        "The class action remains the central vehicle for nationwide remedies.\u{E000}20\u{E001}",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    body.font_ratio_page_ref = 1.0;
    body.in_footnote_zone = true;
    body.doc_footnote_state = true;
    body.page_has_footnote_divider = true;
    body.below_footnote_divider = false;
    let mut note = lm2_test_source_line(
        "p4:l45",
        45,
        "20. 145 S. Ct. 2540, 2548 (2025).",
        0.81,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    note.font_ratio_page_ref = 0.81;
    note.in_footnote_zone = true;
    note.page_has_footnote_divider = true;
    note.below_footnote_divider = true;
    let decoded = vec![
        (body.clone(), Lm2Action::Marginalia),
        (note.clone(), Lm2Action::Marginalia),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: body.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref_with_note_start(
                &note,
                LiquidBlockRole::Marginalia,
                true,
            )],
        },
    ];

    assert_eq!(
        apply_above_note_body_marginalia_rescue(&mut blocks, &sources, &decoded),
        1
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
}

#[test]
fn explicit_divider_overrides_stale_learned_note_state_above_it() {
    let mut body = lm2_test_source_line(
        "p4:l42",
        42,
        "ordinary body line",
        1.0,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    body.page_height = 792.0;
    body.top = 277.0;
    body.page_has_footnote_divider = true;
    body.below_footnote_divider = false;
    body.in_footnote_zone = true;
    body.doc_footnote_state = true;

    assert!(!lm2_low_footnote_zone_evidence(&body));
    body.below_footnote_divider = true;
    assert!(lm2_low_footnote_zone_evidence(&body));
}

#[test]
fn d1_runtime_footer_artifact_overlay_preserves_existing_hide_noise() {
    let candidate = lm2_test_source_line(
        "p0:l30",
        30,
        "doi: 10.1234/test",
        0.88,
        false,
        Some(LiquidBlockRole::Noise),
    );
    let mut decoded = vec![(candidate, Lm2Action::HideNoise)];

    apply_d1_runtime_footer_artifact_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn final_assembly_guard_isolates_running_header_after_role_overlays() {
    let mut line = lm2_test_source_line(
        "p7:l0",
        0,
        "8 HARVARD LAW REVIEW FORUM [Vol. 139:1",
        0.79,
        false,
        Some(LiquidBlockRole::Noise),
    );
    line.page_index = 7;
    line.page_height = 792.0;
    line.top = 710.8;
    line.bottom = 695.9;
    let mut decoded = vec![(line, Lm2Action::Marginalia)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn final_assembly_guard_isolates_document_repeated_edge_text() {
    let mut line = lm2_test_source_line("p5:l0", 0, "JOURNAL OF PRIVATE LAW", 0.82, false, None);
    line.page_index = 5;
    line.doc_repeated_edge_text = true;
    line.doc_repeated_top_edge = true;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn final_assembly_guard_isolates_repository_footer_at_page_edge() {
    let mut line = lm2_test_source_line(
        "p2:l45",
        45,
        "Electronic copy available at: https://ssrn.com/abstract=4543803",
        0.84,
        false,
        Some(LiquidBlockRole::Noise),
    );
    line.page_index = 2;
    line.page_height = 792.0;
    line.top = 19.05;
    line.bottom = 7.89;
    let mut decoded = vec![(line, Lm2Action::Marginalia)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn final_assembly_guard_isolates_footnote_divider() {
    let line = lm2_test_source_line(
        "p15:l30",
        30,
        &"\u{2013}".repeat(61),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
}

#[test]
fn final_assembly_guard_restores_layout_confirmed_note_continuation() {
    let mut line = lm2_test_source_line(
        "p15:l48",
        48,
        "class); id. at 119 (discussing a related example).",
        0.73,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    line.page_index = 15;
    line.in_footnote_zone = true;
    line.font_ratio_page_ref = 0.73;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn final_assembly_guard_preserves_non_edge_available_at_citation() {
    let mut line = lm2_test_source_line(
        "p2:l20",
        20,
        "The appendix is available at: https://example.edu/report.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    line.page_height = 792.0;
    line.top = 420.0;
    line.bottom = 408.0;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Paragraph));
}

#[test]
fn final_assembly_guard_restores_short_outline_heading() {
    let mut line = lm2_test_source_line(
        "p1:l12",
        12,
        "I. UCC ARTICLE 4A AND THE UCC DRAFTING PROCESS",
        1.0,
        false,
        Some(LiquidBlockRole::Noise),
    );
    line.page_index = 1;
    line.page_height = 720.0;
    line.top = 470.0;
    line.bottom = 458.0;
    let mut decoded = vec![(line, Lm2Action::HideNoise)];

    apply_final_assembly_safety_guards(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Heading));
}

#[test]
fn repository_cover_guard_drops_cover_and_preserves_spilled_article_title() {
    let mut works = lm2_test_source_line(
        "p0:l5",
        5,
        "Follow this and additional works at: https://example.edu/repository",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    works.page_index = 0;
    let mut citation = lm2_test_source_line(
        "p0:l6",
        6,
        "Recommended Citation Jane Scholar, Important Article",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    citation.page_index = 0;
    let mut spill = lm2_test_source_line(
        "p1:l0",
        0,
        "For more information, please contact repository@example.edu. THE IMPORTANT ARTICLE TITLE",
        1.2,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    spill.page_index = 1;
    let mut decoded = vec![
        (works, Lm2Action::Keep),
        (citation, Lm2Action::Keep),
        (spill, Lm2Action::HideNoise),
    ];

    assert_eq!(apply_repository_cover_guard(&mut decoded), 3);
    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[1].1, Lm2Action::HideNoise);
    assert_eq!(decoded[2].1, Lm2Action::Keep);
    assert_eq!(decoded[2].0.text, "THE IMPORTANT ARTICLE TITLE");
    assert_eq!(decoded[2].0.role_hint, Some(LiquidBlockRole::Heading));
}

#[test]
fn repository_recommended_citation_supplies_clean_title_spelling() {
    let mut works = lm2_test_source_line(
        "p0:l5",
        5,
        "Follow this and additional works at: https://example.edu/repository",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    works.page_index = 0;
    let mut label = lm2_test_source_line(
        "p0:l14",
        14,
        "Recommended Citation",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    label.page_index = 0;
    let mut citation_one = lm2_test_source_line(
        "p0:l15",
        15,
        "Paul S. Turner, The UCC Drafting Process and Six Questions about Article 4A: Is There a Need for",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    citation_one.page_index = 0;
    let mut citation_two = lm2_test_source_line(
        "p0:l16",
        16,
        "Revisions to the Uniform Funds Transfers Law, 28 Loy. L.A. L. Rev. 351 (1994).",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    citation_two.page_index = 0;
    let mut available = lm2_test_source_line(
        "p0:l17",
        17,
        "Available at: https://example.edu/article",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    available.page_index = 0;
    let decoded = vec![
        (works, Lm2Action::Keep),
        (label, Lm2Action::Keep),
        (citation_one, Lm2Action::Keep),
        (citation_two, Lm2Action::Keep),
        (available, Lm2Action::Keep),
    ];
    let covers = repository_cover_pages(&decoded);

    let title = repository_citation_title(&decoded, &covers).unwrap();

    assert_eq!(
        title,
        "The UCC Drafting Process and Six Questions about Article 4A: Is There a Need for Revisions to the Uniform Funds Transfers Law"
    );
    assert!(
        title_word_overlap(
            &title,
            "THE UCC DRAFJTING PROCESS AND SIX QUESTIONS ABOUT ARTICLE 4A: IS THERE A NEED FOR REVISIONS TO THE UNIFORM FUNDS TRANSFERS LAW?"
        ) > 0.8
    );
}

#[test]
fn explicit_endnote_section_guard_recovers_full_page_notes() {
    let mut page_label =
        lm2_test_source_line("p1:l0", 0, "14", 0.82, true, Some(LiquidBlockRole::Noise));
    page_label.page_index = 1;
    let mut note_one =
        lm2_test_source_line("p1:l1", 1, "1", 1.0, false, Some(LiquidBlockRole::Noise));
    note_one.page_index = 1;
    let mut note_one_text = lm2_test_source_line(
        "p1:l2",
        2,
        "See U.C.C. art. 4A prefatory note.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    note_one_text.page_index = 1;
    note_one_text.doc_note_marker = 55;
    let mut decoded = vec![
        (
            lm2_test_source_line(
                "p0:l0",
                0,
                "The article cites one.1 Then two.2 Then three.3 Then four.4",
                1.0,
                false,
                Some(LiquidBlockRole::Paragraph),
            ),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l1", 1, "Footnotes", 1.0, false, None),
            Lm2Action::Keep,
        ),
        (page_label, Lm2Action::HideNoise),
        (note_one, Lm2Action::HideNoise),
        (note_one_text, Lm2Action::Keep),
    ];
    for marker in 2..=4 {
        let mut line = lm2_test_source_line(
            &format!("p1:l{}", marker * 2),
            marker * 2,
            &marker.to_string(),
            1.0,
            false,
            Some(LiquidBlockRole::Noise),
        );
        line.page_index = 1;
        decoded.push((line, Lm2Action::HideNoise));
        let mut text = lm2_test_source_line(
            &format!("p1:l{}", marker * 2 + 1),
            marker * 2 + 1,
            "Id.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        text.page_index = 1;
        decoded.push((text, Lm2Action::Keep));
    }

    let recovered = apply_explicit_endnote_section_guard(&mut decoded);

    assert_eq!(recovered, 8);
    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert!(
        decoded[0]
            .0
            .text
            .contains(&format!("one.{CALLOUT_START}1{CALLOUT_END}"))
    );
    assert!(
        decoded[0]
            .0
            .text
            .contains(&format!("four.{CALLOUT_START}4{CALLOUT_END}"))
    );
    assert_eq!(decoded[1].1, Lm2Action::HideNoise);
    assert_eq!(decoded[2].1, Lm2Action::HideNoise);
    for row in decoded.iter().skip(3) {
        assert_eq!(row.1, Lm2Action::Marginalia);
        assert_eq!(row.0.role_hint, Some(LiquidBlockRole::Marginalia));
    }
    assert_eq!(decoded[4].0.doc_note_marker, 0);
}

#[test]
fn explicit_endnote_section_guard_requires_a_monotone_sequence() {
    let mut decoded = vec![
        (
            lm2_test_source_line("p0:l0", 0, "Footnotes", 1.0, false, None),
            Lm2Action::Keep,
        ),
        (
            lm2_test_source_line("p0:l1", 1, "1", 1.0, false, None),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line("p0:l2", 2, "200", 1.0, false, None),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line("p0:l3", 3, "2", 1.0, false, None),
            Lm2Action::HideNoise,
        ),
        (
            lm2_test_source_line("p0:l4", 4, "3", 1.0, false, None),
            Lm2Action::HideNoise,
        ),
    ];

    assert_eq!(apply_explicit_endnote_section_guard(&mut decoded), 0);
    assert_eq!(decoded[0].1, Lm2Action::Keep);
}

#[test]
fn d1_runtime_safe_numeric_note_overlay_recovers_later_page_note_start() {
    let mut line = lm2_test_source_line(
        "p8:l11",
        11,
        "13. Woman Records Racist Coronavirus Rant While on Subway, CNN (Feb. 21, 2020),",
        0.86,
        false,
        None,
    );
    line.page_index = 8;
    line.bottom = 471.0;
    line.top = 483.0;
    line.page_height = 720.0;
    line.font_ratio_doc = 0.86;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn d1_runtime_safe_numeric_note_overlay_rejects_short_plain_note_shape() {
    let mut line = lm2_test_source_line(
        "p8:l11",
        11,
        "13. Short plain body-like line",
        0.86,
        false,
        None,
    );
    line.page_index = 8;
    line.bottom = 471.0;
    line.top = 483.0;
    line.page_height = 720.0;
    line.font_ratio_doc = 0.86;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn footnote_monotone_overlay_recovers_cross_page_note_start() {
    let mut line = lm2_test_source_line(
        "p8:l11",
        11,
        "23. See Smith, supra note.",
        0.88,
        false,
        None,
    );
    line.page_index = 8;
    line.bottom = 0.43;
    line.top = 0.45;
    line.page_has_footnote_divider = true;
    line.doc_note_marker = 23;
    line.doc_note_marker_first_on_page = true;
    line.doc_note_marker_mid_sequence_page = true;
    line.doc_note_marker_follows_previous_page = true;
    line.doc_note_marker_page_delta = 1;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn footnote_monotone_overlay_recovers_immediate_continuation() {
    let mut anchor = lm2_test_source_line(
        "p8:l11",
        11,
        "23. See Smith v. Jones, 123 U.S. 45, 51 (2020).",
        0.88,
        false,
        None,
    );
    anchor.page_index = 8;
    anchor.bottom = 0.43;
    anchor.top = 0.45;
    anchor.page_has_footnote_divider = true;
    anchor.doc_note_marker = 23;
    let mut continuation = lm2_test_source_line(
        "p8:l12",
        12,
        "explaining the same equitable doctrine in later federal courts.",
        0.88,
        false,
        None,
    );
    continuation.page_index = 8;
    continuation.bottom = 0.40;
    continuation.top = 0.42;
    continuation.doc_footnote_continuation = true;
    let mut decoded = vec![
        (anchor, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
    ];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn footnote_monotone_overlay_rejects_body_numbered_prose() {
    let mut line = lm2_test_source_line(
        "p8:l11",
        11,
        "23. This section explains the doctrine in ordinary body prose.",
        1.0,
        false,
        None,
    );
    line.page_index = 8;
    line.bottom = 0.62;
    line.top = 0.64;
    line.doc_note_marker = 23;
    line.doc_note_marker_first_on_page = true;
    line.doc_note_marker_mid_sequence_page = true;
    line.doc_note_marker_follows_previous_page = true;
    line.doc_note_marker_page_delta = 1;
    let mut decoded = vec![(line, Lm2Action::Keep)];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert_eq!(decoded[0].0.role_hint, None);
}

#[test]
fn footnote_monotone_overlay_recovers_gap_marker_between_known_notes() {
    let mut note22 = lm2_test_source_line(
        "p4:l20",
        20,
        "22. Earlier note already decoded as marginalia.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    note22.doc_note_marker = 22;
    note22.below_footnote_divider = true;
    let mut note23 = lm2_test_source_line(
        "p4:l21",
        21,
        "23. Missing cited source that the first decode kept.",
        0.86,
        false,
        None,
    );
    note23.doc_note_marker = 23;
    note23.below_footnote_divider = true;
    note23.page_has_footnote_divider = true;
    note23.top = 390.0;
    note23.bottom = 378.0;
    note23.page_height = 1000.0;
    let mut note24 = lm2_test_source_line(
        "p4:l22",
        22,
        "24. Later note already decoded as marginalia.",
        0.86,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    note24.doc_note_marker = 24;
    note24.below_footnote_divider = true;
    let mut decoded = vec![
        (note22, Lm2Action::Marginalia),
        (note23, Lm2Action::Keep),
        (note24, Lm2Action::Marginalia),
    ];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn footnote_monotone_overlay_recovers_body_marker_matched_note() {
    let mut body = lm2_test_source_line(
        "p0:l10",
        10,
        "The court adopted the same rule in later cases.\u{00B2}\u{00B3}",
        1.0,
        false,
        None,
    );
    body.right = 520.0;
    body.page_width = 600.0;
    let mut note = lm2_test_source_line(
        "p0:l45",
        45,
        "23. Source explaining the later line of authority.",
        0.86,
        false,
        None,
    );
    note.doc_note_marker = 23;
    note.below_footnote_divider = true;
    note.page_has_footnote_divider = true;
    note.top = 370.0;
    note.bottom = 358.0;
    note.page_height = 1000.0;
    let mut decoded = vec![(body, Lm2Action::Keep), (note, Lm2Action::Keep)];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn footnote_monotone_overlay_rejects_body_marker_without_note_geometry() {
    let mut body = lm2_test_source_line(
        "p0:l10",
        10,
        "The court adopted the same rule in later cases.\u{00B2}\u{00B3}",
        1.0,
        false,
        None,
    );
    body.right = 520.0;
    body.page_width = 600.0;
    let mut false_note = lm2_test_source_line(
        "p0:l11",
        11,
        "23. Ordinary numbered body paragraph, not a footnote.",
        1.0,
        false,
        None,
    );
    false_note.doc_note_marker = 23;
    false_note.top = 690.0;
    false_note.bottom = 678.0;
    false_note.page_height = 1000.0;
    let mut decoded = vec![(body, Lm2Action::Keep), (false_note, Lm2Action::Keep)];

    apply_footnote_monotone_overlay(&mut decoded, &[]);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn open_footnote_carryover_overlay_recovers_dividerless_continuation() {
    let mut tail = lm2_test_source_line(
        "p0:l40",
        40,
        "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    tail.page_index = 0;
    tail.bottom = 0.42;
    tail.top = 0.44;
    tail.in_footnote_zone = true;
    tail.doc_note_marker = 23;

    let mut continuation = lm2_test_source_line(
        "p1:l1",
        1,
        "later courts followed the same rule in closely related cases.",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    continuation.page_index = 1;
    continuation.bottom = 0.70;
    continuation.top = 0.72;
    continuation.page_has_footnote_divider = false;
    continuation.in_footnote_zone = false;
    continuation.right = 0.88;

    let mut decoded = vec![
        (tail, Lm2Action::Marginalia),
        (continuation, Lm2Action::Keep),
    ];

    assert!(!open_footnote_carryover_reject_line(&decoded[0].0));
    assert!(open_footnote_carryover_tail_candidate(
        &decoded[0].0,
        decoded[0].1
    ));
    assert!(!open_footnote_carryover_has_terminal_punctuation(
        &decoded[0].0.text
    ));
    assert!(open_footnote_carryover_page_tail_state(&decoded, &[0]).is_some());
    assert!(open_footnote_carryover_candidate(&decoded[1].0));
    apply_open_footnote_carryover_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
}

#[test]
fn preceding_small_font_note_continuation_is_not_spliced_into_body() {
    let mut decoded = Vec::new();
    let mut body = lm2_test_source_line(
        "p16:l32",
        32,
        "their customers as the drafters thought fair, and numerous provisions",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 16;
    decoded.push((body, Lm2Action::Keep));
    for line_index in 33..=38 {
        let mut continuation = lm2_test_source_line(
            &format!("p16:l{line_index}"),
            line_index,
            "continued small-font note prose from the preceding page",
            0.75,
            false,
            None,
        );
        continuation.page_index = 16;
        decoded.push((continuation, Lm2Action::Keep));
    }
    let mut note = lm2_test_source_line(
        "p16:l39",
        39,
        "39. U.C.C. section 4A-203(a)(1).",
        0.75,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    note.page_index = 16;
    note.in_footnote_zone = true;
    decoded.push((note, Lm2Action::Marginalia));

    assert_eq!(
        apply_preceding_small_font_note_continuation_guard(&mut decoded),
        6
    );
    assert_eq!(decoded[0].1, Lm2Action::Keep);
    assert!(decoded[1..=6].iter().all(|(line, action)| {
        *action == Lm2Action::Marginalia
            && line.role_hint == Some(LiquidBlockRole::Footnote)
            && line.in_footnote_zone
            && line.doc_footnote_continuation
    }));
}

#[test]
fn open_footnote_carryover_overlay_stops_at_expected_marker() {
    let mut tail = lm2_test_source_line(
        "p0:l40",
        40,
        "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    tail.page_index = 0;
    tail.bottom = 0.42;
    tail.top = 0.44;
    tail.in_footnote_zone = true;
    tail.doc_note_marker = 23;

    let mut next_marker = lm2_test_source_line(
        "p1:l1",
        1,
        "24. A new footnote starts on this page.",
        0.88,
        false,
        None,
    );
    next_marker.page_index = 1;
    next_marker.doc_note_marker = 24;
    next_marker.in_footnote_zone = true;

    let mut following = lm2_test_source_line(
        "p1:l2",
        2,
        "ordinary text after the new note marker should not be captured.",
        0.88,
        false,
        None,
    );
    following.page_index = 1;
    following.in_footnote_zone = true;

    let mut decoded = vec![
        (tail, Lm2Action::Marginalia),
        (next_marker, Lm2Action::Keep),
        (following, Lm2Action::Keep),
    ];

    apply_open_footnote_carryover_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[2].1, Lm2Action::Keep);
}

#[test]
fn open_footnote_carryover_overlay_rejects_body_font_resume() {
    let mut tail = lm2_test_source_line(
        "p0:l40",
        40,
        "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
        0.88,
        false,
        Some(LiquidBlockRole::Footnote),
    );
    tail.page_index = 0;
    tail.bottom = 0.42;
    tail.top = 0.44;
    tail.in_footnote_zone = true;
    tail.doc_note_marker = 23;

    let mut body = lm2_test_source_line(
        "p1:l1",
        1,
        "This section resumes the ordinary argument in body-sized prose.",
        1.0,
        false,
        None,
    );
    body.page_index = 1;
    body.bottom = 0.70;
    body.top = 0.72;
    body.page_has_footnote_divider = false;
    body.in_footnote_zone = false;

    let mut decoded = vec![(tail, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

    apply_open_footnote_carryover_overlay(&mut decoded);

    assert_eq!(decoded[1].1, Lm2Action::Keep);
    assert_eq!(decoded[1].0.role_hint, None);
}

#[test]
fn d1_runtime_zerospend_cue_rejects_body_prose_false_positives() {
    assert!(!d1_runtime_zerospend_cue(
        "in the city of London, by which a broker making a contract was held per\u{0002}"
    ));
    assert!(!d1_runtime_zerospend_cue(
        "sonally liable as purchaser, if he did not at the time of the contract disclose"
    ));
    assert!(!d1_runtime_zerospend_cue(
        "as explaining the language of the written contract, or adding to it a tacitly"
    ));
    assert!(!d1_runtime_zerospend_cue(
        "child, and subsequently, were inadmissible to prove it illegitimate. Ibid."
    ));
    assert!(!d1_runtime_zerospend_cue("EN BANC 1 (2023)."));
}

#[test]
fn d1_runtime_zerospend_cue_accepts_legal_citation_rows() {
    assert!(d1_runtime_zerospend_cue(
        "45. Koons v. Platkin, 673 F. Supp. 3d 515, 620 (D.N.J. 2023)."
    ));
    assert!(d1_runtime_zerospend_cue(
        "Weddings: Domicile, Public Policy, and Inequality in Family Law, 2014 Mich. St. L. Rev."
    ));
}

#[test]
fn lm2_source_signature_changes_when_pymupdf_grouping_toggle_changes() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("lawpdf-lm2-signature-toggle-test.pdf");
    std::fs::write(&path, b"%PDF-1.4\n%test\n").expect("temp pdf");

    let pages = vec!["page one".to_owned(), "page two".to_owned()];
    let signature = |use_pymupdf_blocks,
                     pp_footnote_region_membership,
                     marker_decoder_prior,
                     small_font_decoder_prior,
                     small_font_sequence_prior,
                     anchored_marginalia_flow_guard,
                     body_preservation_guard,
                     action_neutral_blocksplit,
                     toc_overlay,
                     front_matter_guard,
                     marginalia_preservation_guard,
                     d1_runtime_zerospend_overlay: bool,
                     d1_runtime_continuation_overlay,
                     d1_runtime_immediate_continuation_overlay,
                     d1_runtime_sandwiched_continuation_overlay,
                     d1_runtime_wide_sandwich_overlay,
                     d1_runtime_safe_numeric_note_overlay,
                     d1_runtime_post_wide_cue_overlay,
                     d1_runtime_postcue_citation_next1_overlay,
                     d1_runtime_near8_cue_overlay,
                     d1_runtime_wide_divider_guard_overlay,
                     d1_runtime_geometric_zone_overlay,
                     d1_runtime_footer_artifact_overlay,
                     page_object_tuned_overlay,
                     start_score_scale,
                     transition_score_scale| {
        let d1_runtime_zerospend_overlay_version =
            d1_runtime_zerospend_overlay.then_some(LM2_D1_RUNTIME_ZEROSPEND_OVERLAY_VERSION);
        lm2_source_signature(
            &path,
            &pages,
            "model-a",
            None,
            None,
            None,
            use_pymupdf_blocks,
            pp_footnote_region_membership,
            marker_decoder_prior,
            small_font_decoder_prior,
            small_font_sequence_prior,
            anchored_marginalia_flow_guard,
            body_preservation_guard,
            action_neutral_blocksplit,
            toc_overlay,
            front_matter_guard,
            marginalia_preservation_guard,
            d1_runtime_zerospend_overlay,
            d1_runtime_zerospend_overlay_version,
            d1_runtime_continuation_overlay,
            d1_runtime_immediate_continuation_overlay,
            d1_runtime_sandwiched_continuation_overlay,
            d1_runtime_wide_sandwich_overlay,
            d1_runtime_safe_numeric_note_overlay,
            d1_runtime_post_wide_cue_overlay,
            d1_runtime_postcue_citation_next1_overlay,
            d1_runtime_near8_cue_overlay,
            d1_runtime_wide_divider_guard_overlay,
            d1_runtime_geometric_zone_overlay,
            d1_runtime_footer_artifact_overlay,
            false,
            false,
            false,
            false,
            page_object_tuned_overlay,
            start_score_scale,
            transition_score_scale,
        )
    };
    let without = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with = signature(
        true, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_pp_footnote_membership = signature(
        false, true, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_marker_prior = signature(
        false, false, true, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_small_font_prior = signature(
        false, false, false, true, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_small_font_sequence_prior = signature(
        false, false, false, false, true, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_anchored_flow_guard = signature(
        false, false, false, false, false, true, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_blocksplit = signature(
        false, false, false, false, false, false, false, true, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_toc_overlay = signature(
        false, false, false, false, false, false, false, false, true, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_front_matter_guard = signature(
        false, false, false, false, false, false, false, false, false, true, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_marginalia_preservation_guard = signature(
        false, false, false, false, false, false, false, false, false, false, true, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_zerospend_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, true, false,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_continuation_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, true,
        false, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_immediate_continuation_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        true, false, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_sandwiched_continuation_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, true, false, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_wide_sandwich_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, true, false, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_safe_numeric_note_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, true, false, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_post_wide_cue_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, true, false, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_postcue_citation_next1_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, true, false, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_near8_cue_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, true, false, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_wide_divider_guard_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, true, false, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_geometric_zone_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, true, false, false, 1.0, 1.0,
    );
    let with_d1_runtime_footer_artifact_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, true, 1.0, 1.0,
    );
    let with_page_object_tuned_overlay = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, true, 1.0, 1.0,
    );
    let with_decoder_scale = signature(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false, false, false, false, false, false, false, 3.0, 3.0,
    );
    let with_static_overlay = lm2_source_signature(
        &path,
        &pages,
        "model-a",
        None,
        None,
        Some("a55:test"),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        1.0,
        1.0,
    );
    let with_context_twopass = lm2_source_signature(
        &path,
        &pages,
        "model-a",
        Some(LM2_CONTEXT_TWOPASS_VERSION),
        None,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        1.0,
        1.0,
    );

    assert_ne!(without, with);
    assert_ne!(without, with_pp_footnote_membership);
    assert_ne!(without, with_marker_prior);
    assert_ne!(without, with_small_font_prior);
    assert_ne!(without, with_small_font_sequence_prior);
    assert_ne!(without, with_anchored_flow_guard);
    assert_ne!(without, with_blocksplit);
    assert_ne!(without, with_toc_overlay);
    assert_ne!(without, with_front_matter_guard);
    assert_ne!(without, with_marginalia_preservation_guard);
    assert_ne!(without, with_d1_runtime_zerospend_overlay);
    assert_ne!(without, with_d1_runtime_continuation_overlay);
    assert_ne!(without, with_d1_runtime_immediate_continuation_overlay);
    assert_ne!(without, with_d1_runtime_sandwiched_continuation_overlay);
    assert_ne!(without, with_d1_runtime_wide_sandwich_overlay);
    assert_ne!(without, with_d1_runtime_safe_numeric_note_overlay);
    assert_ne!(without, with_d1_runtime_post_wide_cue_overlay);
    assert_ne!(without, with_d1_runtime_postcue_citation_next1_overlay);
    assert_ne!(without, with_d1_runtime_near8_cue_overlay);
    assert_ne!(without, with_d1_runtime_wide_divider_guard_overlay);
    assert_ne!(without, with_d1_runtime_geometric_zone_overlay);
    assert_ne!(without, with_d1_runtime_footer_artifact_overlay);
    assert_ne!(without, with_page_object_tuned_overlay);
    assert_ne!(without, with_decoder_scale);
    assert_ne!(without, with_static_overlay);
    assert_ne!(without, with_context_twopass);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fast_cache_pointer_restores_complete_document_without_geometry() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source_path = std::env::temp_dir().join(format!("lawpdf-fast-cache-{nonce}.pdf"));
    std::fs::write(&source_path, b"fast-cache-fixture").unwrap();
    let source_signature = format!("fast-cache-test-{nonce}");
    let document = LiquidDocument {
        title: "Cached".to_owned(),
        blocks: vec![LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "Cached body".to_owned(),
            label: None,
        }],
        article_spans: Vec::new(),
        block_source_lines: vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: Some("p0:l0".to_owned()),
                page_index: 0,
                line_index: 0,
                text: "Cached body".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        }],
        footnote_links: Vec::new(),
        footnote_link_integrity: None,
        profile: Some(lm2_profile()),
        noise_lines_removed: 0,
        llm_used: false,
        llm_provider: Some("LM2".to_owned()),
        deep_liquid_used: false,
        deep_liquid_model: None,
        warnings: Vec::new(),
        source_signature: source_signature.clone(),
    };
    save_cached_lm2_document(&document).unwrap();
    save_fast_cached_lm2_document(
        &source_path,
        false,
        false,
        Lm2RuntimeChoice::CatBoost,
        &document,
    )
    .unwrap();

    let loaded = load_fast_cached_liquid_mode2_document(
        &source_path,
        false,
        false,
        Lm2RuntimeChoice::CatBoost,
    )
    .unwrap();
    assert_eq!(loaded.title, "Cached");
    assert_eq!(loaded.source_signature, source_signature);

    let catboost_pointer =
        fast_cache::lm2_fast_cache_path(&source_path, false, false, Lm2RuntimeChoice::CatBoost)
            .unwrap();
    let fasttab_pointer =
        fast_cache::lm2_fast_cache_path(&source_path, false, false, Lm2RuntimeChoice::FastTab)
            .unwrap();
    assert_ne!(catboost_pointer, fasttab_pointer);

    if let Some(path) = lm2_cache_path(&source_signature) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(path) =
        fast_cache::lm2_fast_cache_path(&source_path, false, false, Lm2RuntimeChoice::CatBoost)
    {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_file(source_path);
}

#[test]
fn sentineled_note_start_preserves_note_identity() {
    assert_eq!(
        sentineled_note_markers("\u{E000}127\u{E001} See authority."),
        vec![127]
    );
    assert_eq!(
        sentineled_note_markers(
            "citation continuation. \u{E000}127\u{E001} New note. \u{E000}128\u{E001} Next."
        ),
        vec![127, 128]
    );
}

fn body_test_line(id: &str, page: usize, line_index: usize, text: &str) -> DeepLiquidSourceLine {
    let mut line = lm2_test_source_line(id, line_index, text, 1.0, false, None);
    line.page_index = page;
    line.page_width = 100.0;
    line.page_height = 100.0;
    line.left = 20.0;
    line.right = 80.0;
    line.font_height = 10.0;
    line.font_ratio_page_ref = 1.0;
    line.font_ratio_doc = 1.0;
    line.segment_block_id = 7;
    line.segment_block_line_count = 8;
    line.segment_block_shape = "body".to_owned();
    line
}

#[allow(clippy::too_many_arguments)]
fn gapfill_test_line(
    id: &str,
    page: usize,
    line_index: usize,
    text: &str,
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    font_height: f32,
    segment_block_id: usize,
    segment_line_index: usize,
    segment_line_count: usize,
    segment_shape: &str,
) -> DeepLiquidSourceLine {
    let mut line = body_test_line(id, page, line_index, text);
    line.page_width = 442.8;
    line.page_height = 686.88;
    line.left = left;
    line.right = right;
    line.bottom = bottom;
    line.top = top;
    line.font_height = font_height;
    line.font_ratio_doc = font_height / 11.67;
    line.font_ratio_page_ref = font_height / 11.58;
    line.segment_block_id = segment_block_id;
    line.segment_block_line_index = segment_line_index;
    line.segment_block_line_count = segment_line_count;
    line.segment_block_shape = segment_shape.to_owned();
    line
}

fn paragraph_block(text: &str) -> LiquidBlock {
    LiquidBlock {
        role: LiquidBlockRole::Paragraph,
        text: text.to_owned(),
        label: None,
    }
}

#[test]
fn physical_sequential_note_heads_cross_blocks_and_reject_furniture() {
    let note_line = |id: &str, line_index: usize, text: &str| {
        let mut line = lm2_test_source_line(
            id,
            line_index,
            text,
            0.76,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 21;
        line.font_ratio_page_ref = 0.76;
        line.font_ratio_doc = 0.76;
        line.page_has_footnote_divider = true;
        line.below_footnote_divider = true;
        line.in_footnote_zone = true;
        line.segment_block_id = 7;
        line.segment_block_shape = "footnote".to_owned();
        line.segment_block_footnote_like = true;
        line
    };
    let note82 = note_line("p21:l27", 27, "82 Prior note ends.");
    let note83 = note_line("p21:l28", 28, "83");
    let prose83 = note_line("p21:l29", 29, "The next definition continues here.");
    let note84 = note_line("p21:l33", 33, "84 Another definition.");
    let mut page_number = note_line("p21:l34", 34, "85");
    page_number.segment_block_furniture_like = true;
    page_number.centered = true;

    let mut ref82 = line_ref(&note82, LiquidBlockRole::Marginalia);
    ref82.note_markers = vec![82];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note82.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: format!("{} {} {}", note83.text, prose83.text, note84.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: page_number.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![ref82],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&note83, LiquidBlockRole::Marginalia),
                line_ref(&prose83, LiquidBlockRole::Marginalia),
                line_ref(&note84, LiquidBlockRole::Marginalia),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&page_number, LiquidBlockRole::Noise)],
        },
    ];
    let decoded = vec![
        (note82, Lm2Action::Marginalia),
        (note83, Lm2Action::Marginalia),
        (prose83, Lm2Action::Marginalia),
        (note84, Lm2Action::Marginalia),
        (page_number, Lm2Action::HideNoise),
    ];

    assert_eq!(
        apply_physical_sequential_note_head_recovery(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(sources[1].lines[0].note_markers, vec![83]);
    assert_eq!(sources[1].lines[2].note_markers, vec![84]);
    assert!(sources[2].lines[0].note_markers.is_empty());
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert_eq!(blocks.len(), 3);
}

#[test]
fn local_monotone_ascii_callouts_recover_infix_and_same_row_leading_only() {
    let body_line = |id: &str, line_index: usize, text: &str| {
        let mut line = lm2_test_source_line(
            id,
            line_index,
            text,
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        line.page_width = 612.0;
        line.page_height = 792.0;
        line.font_height = 12.0;
        line.font_ratio_page_ref = 1.0;
        line.font_ratio_doc = 1.0;
        line.segment_block_id = 4;
        line.segment_block_shape = "mixed".to_owned();
        line
    };
    let mut body141 = body_line(
        "p53:l11",
        11,
        &format!("A prior proposition.{CALLOUT_START}141{CALLOUT_END}"),
    );
    let mut body142 = body_line("p53:l12", 12, "That leaves the query. 142");
    let body143 = body_line(
        "p53:l13",
        13,
        &format!("The next proposition.{CALLOUT_START}143{CALLOUT_END}"),
    );
    body141.left = 54.0;
    body141.right = 330.0;
    body141.bottom = 500.0;
    body141.top = 512.0;
    body142.left = 330.5;
    body142.right = 390.0;
    body142.bottom = 500.0;
    body142.top = 512.0;

    let note_line = |marker: u16| {
        let mut line = lm2_test_source_line(
            &format!("p53:l{}", 30 + marker as usize % 10),
            30 + marker as usize % 10,
            &format!("{marker} Definition."),
            0.76,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 53;
        line.in_footnote_zone = true;
        line.page_has_footnote_divider = true;
        line.below_footnote_divider = true;
        line
    };
    let notes = [141u16, 142, 143]
        .into_iter()
        .map(note_line)
        .collect::<Vec<_>>();
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: body141.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: body142.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: body143.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body141, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&body142, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&body143, LiquidBlockRole::Paragraph)],
        },
    ];
    for (offset, note) in notes.iter().enumerate() {
        let marker = 141 + offset as u16;
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        });
        let mut note_ref = line_ref(note, LiquidBlockRole::Marginalia);
        note_ref.note_markers = vec![marker];
        sources.push(LiquidBlockSourceLines {
            block_index: 3 + offset,
            lines: vec![note_ref],
        });
    }
    let mut decoded = vec![
        (body141, Lm2Action::Keep),
        (body142, Lm2Action::Marginalia),
        (body143, Lm2Action::Keep),
    ];
    decoded.extend(notes.into_iter().map(|line| (line, Lm2Action::Marginalia)));

    assert_eq!(
        apply_local_monotone_ascii_callout_recovery(&mut blocks, &mut sources, &decoded),
        1
    );
    assert!(
        blocks[1]
            .text
            .contains(&format!("query. {CALLOUT_START}142{CALLOUT_END}"))
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);

    blocks[1].text = "A year 2026 remains ordinary prose.".to_owned();
    sources[1].lines[0].text = blocks[1].text.clone();
    assert_eq!(
        apply_local_monotone_ascii_callout_recovery(&mut blocks, &mut sources, &decoded),
        0
    );
}

#[test]
fn body_paired_bare_note_moves_to_adjacent_definition_block() {
    let mut callout = lm2_test_source_line(
        "p18:l10",
        10,
        &format!("A proposition.{CALLOUT_START}69{CALLOUT_END}"),
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    callout.page_index = 18;
    let mut body = lm2_test_source_line(
        "p18:l13",
        13,
        "The goal is to 69 ascertain intent.",
        1.0,
        false,
        Some(LiquidBlockRole::Paragraph),
    );
    body.page_index = 18;
    let mut marker = lm2_test_source_line(
        "p18:l14",
        14,
        "69",
        0.47,
        false,
        Some(LiquidBlockRole::Marginalia),
    );
    marker.page_index = 18;
    marker.font_ratio_page_ref = 0.47;
    marker.font_ratio_page = 0.61;
    marker.font_ratio_doc = 0.47;
    marker.segment_block_footnote_like = true;
    let mut note = lm2_test_source_line(
        "p18:l15",
        15,
        "For an insightful treatment, see Authority.",
        0.77,
        true,
        None,
    );
    note.page_index = 18;
    note.font_ratio_page_ref = 0.77;
    note.font_ratio_page = 1.0;
    note.font_ratio_doc = 0.77;

    let mut blocks = vec![
        paragraph_block(&callout.text),
        paragraph_block(&body.text),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut marker_ref = line_ref(&marker, LiquidBlockRole::Marginalia);
    marker_ref.note_markers.clear();
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&callout, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&body, LiquidBlockRole::Paragraph), marker_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (callout, Lm2Action::Keep),
        (body, Lm2Action::Keep),
        (marker, Lm2Action::Marginalia),
        (note, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_body_backed_bare_note_head_recovery(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(
        apply_recovered_trailing_note_marker_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].text, "The goal is to ascertain intent.");
    assert!(blocks[2].text.starts_with("69 For an insightful"));
    assert_eq!(sources[2].lines[0].note_markers, vec![69]);
}

#[test]
fn sandwiched_url_rows_continue_the_preceding_numbered_note() {
    let note_line = |id: &str, index: usize, text: &str| {
        let mut line = lm2_test_source_line(
            id,
            index,
            text,
            0.77,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 18;
        line.in_footnote_zone = true;
        line.below_footnote_divider = true;
        line.segment_block_footnote_like = true;
        line
    };
    let head69 = note_line("p18:l14", 14, "69");
    let prose69 = note_line("p18:l15", 15, "A source ending with a URL,");
    let url_one = note_line("p18:l16", 16, "https://example.test/qualify-as\u{0002}");
    let url_two = note_line("p18:l17", 17, "sandwiches/.");
    let head70 = note_line("p18:l18", 18, "70");
    let prose70 = note_line("p18:l19", 19, "The next authority.");
    let mut head69_ref = line_ref(&head69, LiquidBlockRole::Marginalia);
    head69_ref.note_markers = vec![69];
    let mut head70_ref = line_ref(&head70, LiquidBlockRole::Marginalia);
    head70_ref.note_markers = vec![70];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "69 A source ending with a URL,".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: "https://example.test/qualify-as-sandwiches/.".to_owned(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: "70 The next authority.".to_owned(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![head69_ref, line_ref(&prose69, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&url_one, LiquidBlockRole::Noise),
                line_ref(&url_two, LiquidBlockRole::Noise),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![head70_ref, line_ref(&prose70, LiquidBlockRole::Marginalia)],
        },
    ];
    let decoded = vec![
        (head69, Lm2Action::Marginalia),
        (prose69, Lm2Action::Marginalia),
        (url_one, Lm2Action::HideNoise),
        (url_two, Lm2Action::HideNoise),
        (head70, Lm2Action::Marginalia),
        (prose70, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_sandwiched_note_url_continuation_reflow(&mut blocks, &mut sources, &decoded,),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
    assert!(
        sources[1]
            .lines
            .iter()
            .all(|line| line.role == LiquidBlockRole::Marginalia)
    );
}

#[test]
fn terminal_callout_order_and_dotted_version_restorations_are_source_backed() {
    let mut previous = body_test_line("p3:l12", 3, 12, "behind the veil.");
    previous.page_index = 3;
    let mut current = body_test_line("p3:l13", 3, 13, "14 That");
    current.page_index = 3;
    let mut source = LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&previous, LiquidBlockRole::Paragraph),
            line_ref(&current, LiquidBlockRole::Paragraph),
        ],
    };
    source.lines[1].text = "14 That".to_owned();
    let mut blocks = vec![paragraph_block(&format!(
        "Prior.{CALLOUT_START}13{CALLOUT_END} behind the veil. lack of proof.{CALLOUT_START}15{CALLOUT_END} {CALLOUT_START}14{CALLOUT_END} That"
    ))];
    let decoded = vec![(previous, Lm2Action::Keep), (current, Lm2Action::Keep)];
    assert_eq!(
        apply_terminal_out_of_order_callout_fragment_reflow(&mut blocks, &[source], &decoded,),
        1
    );
    assert!(blocks[0].text.contains(&format!(
        "behind the veil.{CALLOUT_START}14{CALLOUT_END} That lack of proof.{CALLOUT_START}15{CALLOUT_END}"
    )));

    let version_source = LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![LiquidSourceLineRef {
            id: Some("p27:l12".to_owned()),
            page_index: 27,
            line_index: 12,
            text: "The software Pangram V3.3.2 classified the comments.".to_owned(),
            role: LiquidBlockRole::Paragraph,
            note_markers: Vec::new(),
        }],
    };
    blocks[0].text = format!(
        "The software Pangram V3.{CALLOUT_START}93{CALLOUT_END}.2 classified the comments. Written.{CALLOUT_START}93{CALLOUT_END}"
    );
    assert_eq!(
        apply_source_backed_dotted_numeric_callout_restoration(&mut blocks, &[version_source],),
        1
    );
    assert!(blocks[0].text.contains("Pangram V3.3.2 classified"));
    assert_eq!(sentineled_note_markers(&blocks[0].text), vec![93]);
}

#[test]
fn final_noise_callout_bridge_restores_gapfill_footnote_98_body_flow() {
    let body_line = |id: &str, line_index: usize, text: &str| {
        let mut line = lm2_test_source_line(
            id,
            line_index,
            text,
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        line.page_index = 29;
        line.page_width = 612.0;
        line.page_height = 792.0;
        line.font_height = 12.0;
        line.font_ratio_page_ref = 1.0;
        line.font_ratio_doc = 1.0;
        line.segment_block_id = 4;
        line.segment_block_line_count = 3;
        line.segment_block_shape = "mixed".to_owned();
        line
    };
    let mut previous = body_line("p29:l8", 8, "Averaging succeeds 55% of");
    let mut hidden = body_line("p29:l9", 9, "the time.");
    let mut callout = body_line(
        "p29:l10",
        10,
        &format!("{CALLOUT_START}98{CALLOUT_END} This is over double the baseline."),
    );
    previous.bottom = 270.0;
    previous.top = 282.0;
    hidden.left = 53.28;
    hidden.right = 112.49;
    hidden.bottom = 249.408;
    hidden.top = 263.688;
    callout.left = 113.52;
    callout.right = 392.13;
    callout.bottom = 249.408;
    callout.top = 263.688;
    let mut note98 = body_line("p29:l20", 20, "98 With three scenarios, chance is 1/3.");
    note98.font_height = 9.0;
    note98.font_ratio_page = 0.76;
    note98.font_ratio_page_ref = 0.76;
    note98.font_ratio_doc = 0.76;
    note98.in_footnote_zone = true;
    note98.page_has_footnote_divider = true;
    note98.below_footnote_divider = true;
    note98.segment_block_id = 9;
    note98.segment_block_shape = "footnote".to_owned();
    note98.segment_block_footnote_like = true;

    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: previous.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: hidden.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: callout.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note98.text.clone(),
            label: None,
        },
    ];
    let mut note_ref = line_ref(&note98, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![98];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&hidden, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&callout, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![note_ref],
        },
    ];
    let decoded = vec![
        (previous, Lm2Action::Keep),
        (hidden, Lm2Action::HideNoise),
        (callout, Lm2Action::Keep),
        (note98, Lm2Action::Marginalia),
    ];

    assert_eq!(
        apply_final_noise_inline_callout_bridge(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains(&format!(
        "55% of the time. {CALLOUT_START}98{CALLOUT_END} This is"
    )));
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
}

#[test]
fn final_caption_continuation_uses_same_segment_style_and_gap() {
    let caption_line = |id: &str, line_index: usize, text: &str, segment: usize| {
        let mut line = lm2_test_source_line(
            id,
            line_index,
            text,
            0.75,
            false,
            Some(LiquidBlockRole::Caption),
        );
        line.page_index = 40;
        line.page_width = 612.0;
        line.page_height = 792.0;
        line.font_height = 9.0;
        line.font_ratio_page_ref = 0.75;
        line.font_ratio_doc = 0.75;
        line.segment_block_id = segment;
        line.segment_block_shape = "body".to_owned();
        line.italic = true;
        line
    };
    let cap = caption_line(
        "p40:l2",
        2,
        "Figure 10: Rankings of the other two models'",
        4,
    );
    let tail = caption_line("p40:l3", 3, "answers. Error bars show uncertainty.", 4);
    let other = caption_line("p40:l4", 4, "Figure 11: A separate figure", 5);
    let mut heading = caption_line("p40:l5", 5, "results and discussion", 6);
    heading.italic = false;
    let decoded = vec![
        (cap.clone(), Lm2Action::HideNoise),
        (tail.clone(), Lm2Action::Keep),
        (other.clone(), Lm2Action::HideNoise),
        (heading.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: cap.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: tail.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: other.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: heading.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&cap, LiquidBlockRole::Caption)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&tail, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&other, LiquidBlockRole::Caption)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&heading, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_caption_continuation_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].role, LiquidBlockRole::Caption);
    assert!(blocks[0].text.contains("models' answers. Error bars"));
    assert_eq!(blocks[1].text, other.text);
}

#[test]
fn final_source_backed_figure_captions_demote_body_references_and_split_fused_prose() {
    let make_line = |id: &str,
                     page: usize,
                     index: usize,
                     text: &str,
                     segment: usize,
                     font_height: f32,
                     italic: bool,
                     role: LiquidBlockRole| {
        let mut line = lm2_test_source_line(id, index, text, 1.0, false, Some(role));
        line.page_index = page;
        line.page_width = 612.0;
        line.page_height = 792.0;
        line.font_height = font_height;
        line.font_ratio_doc = font_height / 12.0;
        line.font_ratio_page_ref = font_height / 12.0;
        line.segment_block_id = segment;
        line.segment_block_shape = "body".to_owned();
        line.italic = italic;
        line
    };
    let body_reference = make_line(
        "p34:l0",
        34,
        0,
        "Figure 6 summarizes the findings.",
        1,
        12.0,
        false,
        LiquidBlockRole::Paragraph,
    );
    let caption = make_line(
        "p34:l1",
        34,
        1,
        "Figure 6: Strict Reconstruction results",
        2,
        9.0,
        true,
        LiquidBlockRole::Caption,
    );
    let body = make_line(
        "p34:l2",
        34,
        2,
        "Roughly 26% of respondents answered correctly.",
        3,
        12.0,
        false,
        LiquidBlockRole::Paragraph,
    );
    let cap10 = make_line(
        "p40:l1",
        40,
        1,
        "Figure 10: Accuracy across all measured agreements and on the hard",
        4,
        9.0,
        true,
        LiquidBlockRole::Caption,
    );
    let cap10_tail = make_line(
        "p40:l2",
        40,
        2,
        "disputes alone. Error bars show confidence intervals.",
        4,
        9.0,
        true,
        LiquidBlockRole::Paragraph,
    );
    let decoded = vec![
        (body_reference.clone(), Lm2Action::Keep),
        (caption.clone(), Lm2Action::HideNoise),
        (body.clone(), Lm2Action::Keep),
        (cap10.clone(), Lm2Action::HideNoise),
        (cap10_tail.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        LiquidBlock {
            role: LiquidBlockRole::Caption,
            text: body_reference.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Caption,
            text: format!("{} {}", caption.text, body.text),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: cap10.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: cap10_tail.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body_reference, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&caption, LiquidBlockRole::Caption),
                line_ref(&body, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&cap10, LiquidBlockRole::Caption)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&cap10_tail, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_source_backed_figure_caption_isolation(&mut blocks, &mut sources, &decoded,),
        4
    );
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].role, LiquidBlockRole::Caption);
    assert_eq!(blocks[1].text, caption.text);
    assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[2].text, body.text);
    assert_eq!(blocks[3].role, LiquidBlockRole::Caption);
    assert!(blocks[3].text.contains("hard disputes alone"));
}

#[test]
fn final_inline_dash_fragment_reflow_restores_exact_gapfill_rows_and_preserves_closed_boundary() {
    let cases = [
        (
            gapfill_test_line(
                "p2:l12",
                2,
                12,
                "a floating price has to be set8",
                194.09,
                329.8408,
                451.174,
                466.83398,
                11.8425,
                3,
                0,
                1,
                "mixed",
            ),
            gapfill_test_line(
                "p2:l13",
                2,
                13,
                "—judges will",
                329.86,
                391.984,
                451.174,
                466.83398,
                12.0,
                4,
                0,
                1,
                "table",
            ),
            gapfill_test_line(
                "p2:l14",
                2,
                14,
                "sometimes supply the missing term.",
                53.304,
                217.25,
                435.454,
                451.114,
                12.0,
                5,
                0,
                1,
                "mixed",
            ),
            "a floating price has to be set\u{E000}8\u{E001}",
        ),
        (
            gapfill_test_line(
                "p9:l22",
                9,
                22,
                "contingency because they know the law will give them a default term34",
                53.304,
                377.85474,
                287.354,
                303.01398,
                11.8539,
                7,
                0,
                1,
                "body",
            ),
            gapfill_test_line(
                "p9:l23", 9, 23, "—", 377.86, 389.74, 287.354, 303.01398, 12.0, 8, 0, 1, "mixed",
            ),
            gapfill_test_line(
                "p9:l24",
                9,
                24,
                "that’s not a “gap”, that’s efficient drafting!—or Richard Posner’s observation",
                53.304,
                389.632,
                271.634,
                287.294,
                11.859,
                9,
                0,
                2,
                "body",
            ),
            "contingency because they know the law will give them a default term\u{E000}34\u{E001}",
        ),
    ];

    for (previous, fragment, current, target_text) in cases {
        let decoded = vec![
            (previous.clone(), Lm2Action::Keep),
            (fragment.clone(), Lm2Action::HideNoise),
            (current.clone(), Lm2Action::Keep),
        ];
        let mut blocks = vec![
            paragraph_block(target_text),
            LiquidBlock {
                role: LiquidBlockRole::Noise,
                text: fragment.text.clone(),
                label: None,
            },
            paragraph_block(&current.text),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&fragment, LiquidBlockRole::Noise)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_final_inline_dash_fragment_reflow(&mut blocks, &mut sources, &decoded),
            1
        );
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].text.contains(&fragment.text));
        assert!(blocks[0].text.ends_with(&current.text));
    }

    let previous = gapfill_test_line(
        "p2:l20",
        2,
        20,
        "This sentence is complete.",
        194.09,
        329.8408,
        451.174,
        466.83398,
        12.0,
        13,
        0,
        1,
        "body",
    );
    let fragment = gapfill_test_line(
        "p2:l21", 2, 21, "—", 329.86, 389.74, 451.174, 466.83398, 12.0, 14, 0, 1, "mixed",
    );
    let current = gapfill_test_line(
        "p2:l22",
        2,
        22,
        "lowercase text from another paragraph.",
        53.304,
        300.0,
        435.454,
        451.114,
        12.0,
        15,
        0,
        1,
        "body",
    );
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (fragment.clone(), Lm2Action::HideNoise),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: fragment.text.clone(),
            label: None,
        },
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&fragment, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_inline_dash_fragment_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 3);
}

#[test]
fn final_same_baseline_fragment_reflow_restores_exact_gapfill_marker_chains() {
    let body = gapfill_test_line(
        "p2:l14",
        2,
        14,
        "sometimes supply the missing term.",
        53.304,
        217.25,
        435.454,
        451.114,
        12.0,
        5,
        0,
        1,
        "mixed",
    );
    let mut marker = gapfill_test_line(
        "p2:l15",
        2,
        15,
        "9",
        217.97,
        221.3108,
        440.91592,
        449.99872,
        6.96,
        7,
        0,
        1,
        "furniture",
    );
    marker.segment_block_footnote_like = true;
    marker.in_footnote_zone = true;
    let continuation = gapfill_test_line(
        "p2:l16",
        2,
        16,
        "Modern jurists have a name for this",
        221.35,
        391.846,
        435.454,
        451.114,
        12.0,
        6,
        0,
        1,
        "table",
    );
    let decoded = vec![
        (body.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::HideNoise),
        (continuation.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&body.text),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: marker.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Table,
            text: continuation.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&body, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&marker, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&continuation, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_same_baseline_fragment_reflow(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(attached_terminal_body_marker(&blocks[0].text), None);
    assert_eq!(blocks[0].text.matches("\u{E000}9\u{E001}").count(), 1);
    assert!(blocks[0].text.ends_with(&continuation.text));

    let first = gapfill_test_line(
        "p63:l11",
        63,
        11,
        "gap filling. 177",
        53.304,
        114.6356,
        442.65402,
        458.31403,
        10.3675,
        1,
        5,
        8,
        "body",
    );
    let second = gapfill_test_line(
        "p63:l12",
        63,
        12,
        "The problem is that consumers’ expectations about what",
        114.62,
        391.736,
        442.65402,
        458.31403,
        12.0,
        1,
        6,
        8,
        "body",
    );
    let later = gapfill_test_line(
        "p63:l19",
        63,
        19,
        "Generative gap filling may produce a separate paragraph.",
        71.304,
        391.9,
        357.214,
        372.874,
        12.0,
        5,
        0,
        3,
        "body",
    );
    let decoded = vec![
        (first.clone(), Lm2Action::Keep),
        (second.clone(), Lm2Action::Keep),
        (later.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&first.text),
        paragraph_block(&second.text),
        paragraph_block(&later.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&first, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&second, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&later, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_same_baseline_fragment_reflow(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.ends_with(&second.text));
    assert_eq!(blocks[1].text, later.text);

    let line_4 = gapfill_test_line(
        "p5:l4",
        5,
        4,
        "superficially plausible as the models it drew on, but it could not be proven.",
        53.304,
        383.02002,
        575.164,
        590.824,
        11.714,
        2,
        0,
        1,
        "body",
    );
    let mut marker_20 = gapfill_test_line(
        "p5:l5",
        5,
        5,
        "20",
        382.9,
        389.60077,
        580.6259,
        589.70874,
        6.96,
        3,
        0,
        1,
        "furniture",
    );
    marker_20.segment_block_footnote_like = true;
    let line_6 = gapfill_test_line(
        "p5:l6",
        5,
        6,
        "One carefully-argued response put it starkly: “no experiment can determine",
        53.304,
        391.76,
        559.564,
        575.224,
        12.0,
        4,
        0,
        3,
        "body",
    );
    let line_9 = gapfill_test_line(
        "p5:l9",
        5,
        9,
        "Here, by redacting the contested text, a new paragraph begins.",
        71.304,
        391.922,
        505.324,
        520.984,
        11.855,
        5,
        0,
        6,
        "body",
    );
    let decoded = vec![
        (line_4.clone(), Lm2Action::Keep),
        (marker_20.clone(), Lm2Action::HideNoise),
        (line_6.clone(), Lm2Action::Keep),
        (line_9.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&format!("{}\u{E000}20\u{E001}", line_4.text)),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: marker_20.text.clone(),
            label: None,
        },
        paragraph_block(&line_6.text),
        paragraph_block(&line_9.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&line_4, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&marker_20, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&line_6, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&line_9, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text.matches("\u{E000}20\u{E001}").count(), 1);
    assert!(blocks[0].text.ends_with(&line_6.text));
    assert_eq!(blocks[1].text, line_9.text);
}

#[test]
fn final_body_coalescer_restores_exact_gapfill_same_role_wraps_and_preserves_list_boundary() {
    let line_22 = gapfill_test_line(
        "p26:l22",
        26,
        22,
        "b. At [CKS]’s option, either the contract price ($8,516)",
        125.3,
        379.982,
        241.634,
        257.294,
        11.807,
        7,
        0,
        4,
        "mixed",
    );
    let line_23 = gapfill_test_line(
        "p26:l23",
        26,
        23,
        "or the current market price ($8,950)",
        143.33,
        303.422,
        225.914,
        241.574,
        12.0,
        7,
        1,
        4,
        "mixed",
    );
    let line_24 = gapfill_test_line(
        "p26:l24",
        26,
        24,
        "c. Only the contract price ($8,516)",
        125.3,
        300.0,
        210.194,
        225.854,
        12.0,
        7,
        2,
        4,
        "mixed",
    );
    let decoded = vec![
        (line_22.clone(), Lm2Action::Keep),
        (line_23.clone(), Lm2Action::Keep),
        (line_24.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&line_22.text),
        paragraph_block(&line_23.text),
        paragraph_block(&line_24.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&line_22, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&line_23, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&line_24, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.ends_with(&line_23.text));
    assert_eq!(blocks[1].text, line_24.text);

    let line_3 = gapfill_test_line(
        "p36:l3",
        36,
        3,
        "Let’s start with the obvious worry that when someone (including AI)",
        80.304,
        391.716,
        567.964,
        583.624,
        12.0,
        1,
        0,
        5,
        "body",
    );
    let line_4 = gapfill_test_line(
        "p36:l4",
        36,
        4,
        "pattern matches, they do so not based on the text itself, but something else.",
        53.304,
        391.644,
        552.364,
        568.024,
        12.0,
        1,
        1,
        5,
        "body",
    );
    let decoded = vec![
        (line_3.clone(), Lm2Action::Keep),
        (line_4.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![paragraph_block(&line_3.text), paragraph_block(&line_4.text)];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&line_3, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&line_4, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.ends_with(&line_4.text));
}

#[test]
fn final_fused_heading_body_split_restores_exact_gapfill_boundary_and_preserves_wrapped_heading() {
    let heading = gapfill_test_line(
        "p64:l28",
        64,
        28,
        "E. Gaps in Generative Gap Filling and the Limits of the Method",
        71.304,
        359.484,
        186.074,
        201.734,
        11.823,
        8,
        0,
        1,
        "body",
    );
    let body = gapfill_test_line(
        "p64:l29",
        64,
        29,
        "We recognize several important limitations to the discussion above.",
        71.304,
        366.766,
        163.124,
        178.784,
        11.343,
        9,
        0,
        1,
        "body",
    );
    let decoded = vec![
        (heading.clone(), Lm2Action::Keep),
        (body.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Heading,
        text: format!("{} {}", heading.text, body.text),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&heading, LiquidBlockRole::Heading),
            line_ref(&body, LiquidBlockRole::Paragraph),
        ],
    }];
    assert_eq!(
        apply_final_fused_heading_body_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].role, LiquidBlockRole::Heading);
    assert_eq!(blocks[0].text, heading.text);
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].text, body.text);

    let mut wrapped = body.clone();
    wrapped.id = "p64:l30".to_owned();
    wrapped.line_index = 30;
    wrapped.text = "and the Limits of the Method".to_owned();
    wrapped.segment_block_id = heading.segment_block_id;
    wrapped.segment_block_line_index = 1;
    wrapped.segment_block_line_count = 2;
    let mut wrapped_heading = heading.clone();
    wrapped_heading.segment_block_line_count = 2;
    let decoded = vec![
        (wrapped_heading.clone(), Lm2Action::Keep),
        (wrapped.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![LiquidBlock {
        role: LiquidBlockRole::Heading,
        text: format!("{} {}", wrapped_heading.text, wrapped.text),
        label: None,
    }];
    let mut sources = vec![LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&wrapped_heading, LiquidBlockRole::Heading),
            line_ref(&wrapped, LiquidBlockRole::Paragraph),
        ],
    }];
    assert_eq!(
        apply_final_fused_heading_body_splits(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 1);
}

#[test]
fn final_body_coalescer_repairs_same_segment_lowercase_and_hyphen_splits() {
    let same = body_test_line("p0:l4", 0, 4, "echoed the same");
    let ideas = body_test_line("p0:l5", 0, 5, "ideas in public.");
    let mut author = body_test_line("p0:l14", 0, 14, "state-conferred author\u{0002}");
    let mut ity = body_test_line("p0:l15", 0, 15, "ity. Member states responded.");
    author.segment_block_id = 8;
    ity.segment_block_id = 8;
    author.bottom = 50.0;
    author.top = 52.0;
    ity.bottom = 46.0;
    ity.top = 54.0;
    let decoded = vec![
        (same.clone(), Lm2Action::Keep),
        (ideas.clone(), Lm2Action::Keep),
        (author.clone(), Lm2Action::Keep),
        (ity.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("echoed the same"),
        paragraph_block("ideas in public."),
        paragraph_block("state-conferred author-"),
        paragraph_block("ity. Member states responded."),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&same, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&ideas, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&author, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&ity, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "echoed the same ideas in public.");
    assert_eq!(
        blocks[1].text,
        "state-conferred authority. Member states responded."
    );
}

#[test]
fn final_body_coalescer_preserves_cross_page_indented_texas_boundary() {
    let mut previous = body_test_line("p0:l40", 0, 40, "The first paragraph ends.");
    let mut current = body_test_line("p1:l1", 1, 1, "A genuinely new paragraph begins.");
    previous.left = 20.0;
    current.left = 23.7;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
    assert!(paragraph_boundary(&previous, &current));
}

#[test]
fn final_body_coalescer_preserves_exact_columbia_marker_290_boundary() {
    let mut previous = body_test_line(
        "p49:l17",
        49,
        17,
        "administrative inefficiencies.\u{E000}290\u{E001}",
    );
    let mut current = body_test_line(
        "p50:l1",
        50,
        1,
        "Should courts nevertheless inquire into the agency's reasons?",
    );
    previous.page_width = 612.0;
    current.page_width = 612.0;
    previous.left = 138.24017;
    current.left = 159.1203;
    current.segment_block_line_index = 0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
    assert!(paragraph_boundary(&previous, &current));
    assert_eq!(blocks[0].text, previous.text);
    assert_eq!(blocks[1].text, current.text);
}

#[test]
fn final_body_coalescer_accepts_cross_page_facing_margin_shifts() {
    for (previous_left, current_left, previous_text, current_text, expected) in [
        (
            16.0,
            11.9,
            "The analysis must therefore",
            "continue on the facing page.",
            "The analysis must therefore continue on the facing page.",
        ),
        (
            11.9,
            16.0,
            "The review board had not completed",
            "its promised investigation.",
            "The review board had not completed its promised investigation.",
        ),
        (
            22.5,
            16.0,
            "The investigation illustr-",
            "ates the stakes of this policy.",
            "The investigation illustrates the stakes of this policy.",
        ),
    ] {
        let mut previous = body_test_line("p0:l40", 0, 40, previous_text);
        let mut current = body_test_line("p1:l2", 1, 2, current_text);
        previous.left = previous_left;
        current.left = current_left;
        let decoded = vec![
            (previous.clone(), Lm2Action::Keep),
            (current.clone(), Lm2Action::Keep),
        ];
        let mut blocks = vec![
            paragraph_block(&previous.text),
            paragraph_block(&current.text),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
            1
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, expected);
    }
}

#[test]
fn displayed_lead_labels_and_font_cliffs_are_final_paragraph_boundaries() {
    let mut previous = body_test_line("p0:l17", 0, 17, "corresponding to three theories:");
    previous.font_height = 9.7;
    let mut label = body_test_line(
        "p0:l18",
        0,
        18,
        "Ratio Decidendi: The holding follows from the outcome.",
    );
    label.left = 16.2;
    label.font_height = 8.6;
    label.font_ratio_doc = 0.88;
    assert!(lm2_displayed_lead_label_boundary(&previous, &label));

    let mut ordinary = label.clone();
    ordinary.text = "One last wrinkle: in this example the prose continues.".to_owned();
    assert!(!lm2_displayed_lead_label_boundary(&previous, &ordinary));

    let mut quote_tail = body_test_line(
        "p0:l21",
        0,
        21,
        "entitling them to relief.\u{E000}65\u{E001}",
    );
    quote_tail.font_height = 8.9;
    quote_tail.font_ratio_doc = 0.91;
    let mut body = body_test_line("p0:l22", 0, 22, "Ironically, the brief omitted the issue.");
    body.font_height = 9.7;
    body.font_ratio_doc = 1.0;
    assert!(lm2_small_font_to_body_cliff_boundary(&quote_tail, &body));
    body.text = "continued quotation without a terminal sentence".to_owned();
    assert!(!lm2_small_font_to_body_cliff_boundary(&quote_tail, &body));
}

#[test]
fn final_body_coalescer_never_reorders_distinct_source_intervals() {
    let late = body_test_line("p0:l24", 0, 24, "court is bound by that holding.");
    let middle = body_test_line("p0:l20", 0, 20, "of the case given the facts");
    let decoded = vec![
        (late.clone(), Lm2Action::Keep),
        (middle.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![paragraph_block(&middle.text), paragraph_block(&late.text)];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&middle, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&late, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks[0].text, middle.text);
    assert_eq!(blocks[1].text, late.text);
    assert_eq!(sources[0].lines[0].id.as_deref(), Some("p0:l20"));
    assert_eq!(sources[1].lines[0].id.as_deref(), Some("p0:l24"));
}

#[test]
fn final_body_coalescer_restores_penn_ratio_source_order() {
    let texts = [
        "Ratio Decidendi: The holding of a precedential case is the legal norm",
        "that follows from the reasoning necessary to the outcome",
        "of the case given the legally salient facts before the court",
        "and the arguments made by the parties or raised by the court.",
        "Legally Salient Factual Characteristics: The holding is defined by",
        "all of the legally salient facts of the case: a subsequent",
        "court is bound by that holding only if the case shares all",
        "of those factual characteristics.",
    ];
    let mut lines = texts
        .iter()
        .enumerate()
        .map(|(offset, text)| {
            body_test_line(&format!("p47:l{}", 18 + offset), 47, 18 + offset, text)
        })
        .collect::<Vec<_>>();
    for (offset, line) in lines.iter_mut().enumerate() {
        line.page_width = 486.0;
        line.page_height = 720.0;
        line.left = 87.4;
        line.bottom = 397.5 - offset as f32 * 12.96;
        line.top = line.bottom + 11.17;
        line.font_height = 8.7;
        line.font_ratio_doc = 0.91;
        line.segment_block_id = 2;
        line.segment_block_line_index = 16 + offset;
        line.segment_block_line_count = 24;
    }
    let decoded = lines
        .iter()
        .cloned()
        .map(|line| (line, Lm2Action::Keep))
        .collect::<Vec<_>>();
    let a_text = format!("{} {}", texts[0], texts[1]);
    let b_text = texts[2..6].join(" ");
    let c_text = texts[6..8].join(" ");
    // Reproduce the native A/C/B regression: the final two blocks are
    // individually text-correct, but their complete source intervals are
    // adjacent and inverted.
    let mut blocks = vec![
        paragraph_block(&a_text),
        paragraph_block(&c_text),
        paragraph_block(&b_text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: lines[0..2]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Paragraph))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: lines[6..8]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Paragraph))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: lines[2..6]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Marginalia))
                .collect(),
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        3
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, texts.join(" "));
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| line.line_index)
            .collect::<Vec<_>>(),
        (18..=25).collect::<Vec<_>>()
    );
}

#[test]
fn final_body_coalescer_restores_exact_penn_cross_page_display_order() {
    let texts = [
        "Ratio Decidendi: The holding of a precedential case is the legal norm (e.g.,",
        "rule or standard) that follows from the reasoning necessary to the outcome",
        "of the case given the legally salient facts before the court and the arguments",
        "made by the parties or raised by the court.\u{E000}173\u{E001}",
        "Legally Salient Factual Characteristics: The holding of a precedential case is a",
        "legal rule defined by all of the legally salient facts of the case: a subsequent",
        "court is bound by that holding only if the case before it shares all of those",
        "factual characteristics.\u{E000}174\u{E001}",
        "Legislative Holdings: The holding of a case is the legal norm (rule or standard)",
        "that a court announces as the basis for its decision, even if that norm reaches",
        "beyond the legally salient facts before the court.\u{E000}175\u{E001}",
    ];
    let mut lines = texts
        .iter()
        .enumerate()
        .map(|(offset, text)| {
            if offset < 8 {
                body_test_line(&format!("p47:l{}", 18 + offset), 47, 18 + offset, text)
            } else {
                body_test_line(&format!("p48:l{}", offset - 7), 48, offset - 7, text)
            }
        })
        .collect::<Vec<_>>();
    for (offset, line) in lines.iter_mut().enumerate() {
        line.page_width = 468.0;
        line.page_height = 720.0;
        line.left = 87.2;
        line.font_height = 8.65;
        line.font_ratio_doc = 0.91;
        if offset < 8 {
            line.segment_block_id = 2;
            line.segment_block_line_index = 16 + offset;
            line.segment_block_line_count = 24;
            line.bottom = 397.5 - offset as f32 * 12.96;
        } else {
            line.segment_block_id = 1;
            line.segment_block_line_index = offset - 8;
            line.segment_block_line_count = 3;
            line.bottom = 631.5 - (offset - 8) as f32 * 12.96;
        }
        line.top = line.bottom + 11.2;
    }
    // Reproduce the late stale-note state from the native trace: immutable
    // segment provenance remains body-shaped even though these rows were
    // relabeled as note continuations before final assembly.
    for line in &mut lines[2..7] {
        line.in_footnote_zone = true;
        line.doc_footnote_continuation = true;
    }
    let decoded = lines
        .iter()
        .enumerate()
        .map(|(offset, line)| {
            let action = if matches!(offset, 2 | 3 | 4 | 5 | 10) {
                Lm2Action::Marginalia
            } else {
                Lm2Action::Keep
            };
            (line.clone(), action)
        })
        .collect::<Vec<_>>();
    let page_47_a = texts[0..2].join(" ");
    let page_47_b = texts[2..6].join(" ");
    let page_47_c = texts[6..8].join(" ");
    let page_48 = texts[8..11].join(" ");
    let mut blocks = vec![
        paragraph_block(&page_47_a),
        paragraph_block(&page_47_c),
        paragraph_block(&format!("{page_47_b} {page_48}")),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: lines[0..2]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Paragraph))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: lines[6..8]
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Paragraph))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: lines[2..6]
                .iter()
                .chain(lines[8..11].iter())
                .enumerate()
                .map(|(offset, line)| {
                    let role = if matches!(offset, 0 | 1 | 2 | 3 | 6) {
                        LiquidBlockRole::Marginalia
                    } else {
                        LiquidBlockRole::Paragraph
                    };
                    line_ref(line, role)
                })
                .collect(),
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        4
    );
    assert_eq!(
        apply_final_source_backed_paragraph_splits(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].text, texts[0..4].join(" "));
    assert_eq!(blocks[1].text, texts[4..8].join(" "));
    assert_eq!(blocks[2].text, page_48);
    assert_eq!(
        sources[0]
            .lines
            .iter()
            .map(|line| (line.page_index, line.line_index))
            .collect::<Vec<_>>(),
        (18..=21)
            .map(|line_index| (47, line_index))
            .collect::<Vec<_>>()
    );
    assert_eq!(sources[1].lines[0].id.as_deref(), Some("p47:l22"));
    assert_eq!(sources[2].lines[0].id.as_deref(), Some("p48:l1"));
}

#[test]
fn final_body_coalescer_rejects_inverted_distinct_reading_streams() {
    let early_a = body_test_line("p0:l10", 0, 10, "The left column states one rule.");
    let early_b = body_test_line("p0:l11", 0, 11, "It ends as a complete paragraph.");
    let late_a = body_test_line("p0:l30", 0, 30, "The right column states another rule.");
    let late_b = body_test_line("p0:l31", 0, 31, "It also ends as a complete paragraph.");
    let decoded = vec![
        (early_a.clone(), Lm2Action::Keep),
        (early_b.clone(), Lm2Action::Keep),
        (late_a.clone(), Lm2Action::Keep),
        (late_b.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(
            "The right column states another rule. It also ends as a complete paragraph.",
        ),
        paragraph_block("The left column states one rule. It ends as a complete paragraph."),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&late_a, LiquidBlockRole::Paragraph),
                line_ref(&late_b, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&early_a, LiquidBlockRole::Paragraph),
                line_ref(&early_b, LiquidBlockRole::Paragraph),
            ],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert!(blocks[0].text.starts_with("The right column"));
    assert!(blocks[1].text.starts_with("The left column"));
}

#[test]
fn final_body_coalescer_rejects_sorted_refs_with_reversed_duke_text() {
    let early_a = body_test_line("p0:l20", 0, 20, "The first Duke sentence carries");
    let early_b = body_test_line("p0:l21", 0, 21, "into the next source interval and");
    let late_a = body_test_line("p0:l22", 0, 22, "continues in its proper reading");
    let late_b = body_test_line("p0:l23", 0, 23, "order before the paragraph ends.");
    let decoded = vec![
        (early_a.clone(), Lm2Action::Keep),
        (early_b.clone(), Lm2Action::Keep),
        (late_a.clone(), Lm2Action::Keep),
        (late_b.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("order before the paragraph ends. continues in its proper reading"),
        paragraph_block("The first Duke sentence carries into the next source interval and"),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&late_a, LiquidBlockRole::Paragraph),
                line_ref(&late_b, LiquidBlockRole::Paragraph),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&early_a, LiquidBlockRole::Paragraph),
                line_ref(&early_b, LiquidBlockRole::Paragraph),
            ],
        },
    ];
    let original = blocks
        .iter()
        .map(|block| block.text.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.clone())
            .collect::<Vec<_>>(),
        original
    );
}

#[test]
fn final_body_coalescer_preserves_duke_scale_merged_text_order() {
    let mut decoded = Vec::new();
    let mut blocks = Vec::new();
    let mut sources = Vec::new();
    for pair in 0..21usize {
        let first_index = pair * 3;
        let second_index = first_index + 1;
        let first_text = format!("Duke interval {pair:02} carries");
        let second_text = format!("continuation {pair:02}.");
        let first = body_test_line(&format!("p0:l{first_index}"), 0, first_index, &first_text);
        let second = body_test_line(
            &format!("p0:l{second_index}"),
            0,
            second_index,
            &second_text,
        );
        let first_block = blocks.len();
        blocks.push(paragraph_block(&first_text));
        sources.push(LiquidBlockSourceLines {
            block_index: first_block,
            lines: vec![line_ref(&first, LiquidBlockRole::Paragraph)],
        });
        let second_block = blocks.len();
        blocks.push(paragraph_block(&second_text));
        sources.push(LiquidBlockSourceLines {
            block_index: second_block,
            lines: vec![line_ref(&second, LiquidBlockRole::Paragraph)],
        });
        decoded.push((first, Lm2Action::Keep));
        decoded.push((second, Lm2Action::Keep));
    }

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        21
    );
    assert_eq!(blocks.len(), 21);
    for pair in 0..21usize {
        assert_eq!(
            blocks[pair].text,
            format!("Duke interval {pair:02} carries continuation {pair:02}.")
        );
        let source = sources
            .iter()
            .find(|source| source.block_index == pair)
            .expect("merged block keeps provenance");
        assert_eq!(
            source
                .lines
                .iter()
                .map(|line| line.line_index)
                .collect::<Vec<_>>(),
            vec![pair * 3, pair * 3 + 1]
        );
    }
}

#[test]
fn exact_footnote_continuation_furniture_is_hidden_but_prose_is_not() {
    let exact = body_test_line("p0:l40", 0, 40, "footnote continued on next page");
    let prose = body_test_line(
        "p0:l41",
        0,
        41,
        "The footnote continued on next page because the PDF was reflowed.",
    );
    let mut decoded = vec![(exact, Lm2Action::Marginalia), (prose, Lm2Action::Keep)];
    apply_final_assembly_safety_guards(&mut decoded);
    assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    assert_eq!(decoded[1].1, Lm2Action::Keep);
}

#[test]
fn colon_letter_number_body_rescue_requires_full_same_segment_provenance() {
    let anchor = body_test_line("p0:l20", 0, 20, "include the following:");
    let mut letter = body_test_line("p0:l21", 0, 21, "(f) A board may act only with a quorum.");
    let mut number = body_test_line("p0:l22", 0, 22, "(1) A quorum is a majority.");
    letter.font_height = 9.0;
    number.font_height = 9.0;
    letter.font_ratio_doc = 0.88;
    number.font_ratio_doc = 0.88;
    let mut decoded = vec![
        (anchor.clone(), Lm2Action::Keep),
        (letter.clone(), Lm2Action::Marginalia),
        (number.clone(), Lm2Action::Marginalia),
    ];
    assert_eq!(
        apply_same_segment_statutory_subdivision_body_rescue(&mut decoded),
        2
    );
    assert!(
        decoded[1..]
            .iter()
            .all(|(line, action)| *action == Lm2Action::Keep
                && line.role_hint == Some(LiquidBlockRole::Paragraph))
    );

    letter.segment_block_id = 99;
    let mut negative = vec![
        (anchor, Lm2Action::Keep),
        (letter, Lm2Action::Marginalia),
        (number, Lm2Action::Marginalia),
    ];
    assert_eq!(
        apply_same_segment_statutory_subdivision_body_rescue(&mut negative),
        0
    );
}

#[test]
fn citation_suffix_and_near_rule_cross_page_alternates_are_narrow() {
    assert!(lm2_citation_suffix_cross_page_bridge(
        "285. See Stein v. Buccaneers Ltd.",
        "P\u{2019}ship, 772 F.3d 698, 707 (11th Cir. 2014)"
    ));
    assert!(!lm2_citation_suffix_cross_page_bridge(
        "The company was Buccaneers Ltd.",
        "Partnership doctrine remains unsettled."
    ));

    let mut first = body_test_line(
        "p1:l27",
        1,
        27,
        "serving in a principal office, which negates",
    );
    let mut second = body_test_line("p1:l28", 1, 28, "the Appointments Clause.");
    for line in [&mut first, &mut second] {
        line.font_ratio_page = 0.86;
        line.font_ratio_page_ref = 0.86;
        line.font_ratio_doc = 0.86;
        line.font_height = 9.4;
    }
    first.page_object_thin_horizontal_near_line_count = 1;
    first.dist_to_nearest_rule = 5.0;
    let refs = LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&first, LiquidBlockRole::Marginalia),
            line_ref(&second, LiquidBlockRole::Marginalia),
        ],
    };
    let decoded = vec![
        (first.clone(), Lm2Action::Keep),
        (second.clone(), Lm2Action::Keep),
    ];
    let by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    assert!(lm2_near_rule_cross_page_note_segment(&refs, &by_id));
    first.page_object_thin_horizontal_near_line_count = 0;
    let negative = LiquidBlockSourceLines {
        block_index: 0,
        lines: vec![
            line_ref(&first, LiquidBlockRole::Paragraph),
            line_ref(&second, LiquidBlockRole::Paragraph),
        ],
    };
    let decoded_negative = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
    let by_id_negative = decoded_negative
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    assert!(!lm2_near_rule_cross_page_note_segment(
        &negative,
        &by_id_negative
    ));
}

#[test]
fn final_split_children_inherit_hard_footnote_provenance() {
    let mut line = body_test_line(
        "p1:l14",
        1,
        14,
        "Describe Being Fired, Unfired, and Refired in Five Days",
    );
    line.segment_block_shape = "footnote".to_owned();
    line.segment_block_footnote_like = true;
    line.in_footnote_zone = true;
    line.font_ratio_doc = 0.73;
    line.font_ratio_page_ref = 0.73;
    let decoded = [(line.clone(), Lm2Action::Marginalia)];
    let by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut numbered_ref = line_ref(&line, LiquidBlockRole::Marginalia);
    numbered_ref.note_markers = vec![100];
    let refs = vec![numbered_ref];
    assert_eq!(
        lm2_final_split_child_role(LiquidBlockRole::Paragraph, &refs, &by_id),
        LiquidBlockRole::Marginalia
    );

    let mut body_sized = line.clone();
    body_sized.font_ratio_doc = 1.03;
    let decoded = [(body_sized.clone(), Lm2Action::Keep)];
    let by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let refs = vec![line_ref(&body_sized, LiquidBlockRole::Paragraph)];
    assert_eq!(
        lm2_final_split_child_role(LiquidBlockRole::Marginalia, &refs, &by_id),
        LiquidBlockRole::Paragraph
    );

    let mut body_callout = body_test_line(
        "p44:l35",
        44,
        35,
        "there is a rule requiring party presentation of claims163 and defenses",
    );
    body_callout.font_ratio_doc = 1.03;
    let decoded = [(body_callout.clone(), Lm2Action::Keep)];
    let by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut body_ref = line_ref(&body_callout, LiquidBlockRole::Paragraph);
    body_ref.note_markers = vec![163];
    assert_eq!(
        lm2_final_split_child_role(LiquidBlockRole::Marginalia, &[body_ref], &by_id),
        LiquidBlockRole::Paragraph
    );
    assert!(lm2_uppercase_note_continuation_cue(
        "Describe Being Fired, BUS. INSIDER (Feb. 12, 2025), https://example.test"
    ));
    assert!(lm2_uppercase_note_continuation_cue(
        "See, e.g., 12 U.S.C. § 1812(c)(3) (providing a holdover term)."
    ));
    assert!(!lm2_uppercase_note_continuation_cue(
        "A New Body Paragraph Begins Here"
    ));
}

#[test]
fn final_displayed_body_rescue_restores_statute_and_blockquote_without_merging() {
    let mut anchor = body_test_line("p0:l20", 0, 20, "include the following:");
    let mut letter = body_test_line("p0:l21", 0, 21, "(f) A board may act with a quorum.");
    let mut number = body_test_line("p0:l22", 0, 22, "(1) A quorum is a majority.");
    // The native role stream can carry a stale note-state bit even when
    // the source segmenter proves that all three rows are body display.
    anchor.doc_footnote_state = true;
    number.doc_footnote_state = true;
    letter.font_height = 8.7;
    number.font_height = 8.7;
    assert!(!lm2_hard_body_display_line(&anchor));
    assert!(!lm2_hard_body_display_line(&number));
    assert!(lm2_source_backed_body_display_line(&anchor));
    assert!(lm2_source_backed_body_display_line(&number));
    let decoded = vec![
        (anchor.clone(), Lm2Action::Keep),
        (letter.clone(), Lm2Action::Marginalia),
        (number.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("include the following:"),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: letter.text.clone(),
            label: None,
        },
        paragraph_block(&number.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&anchor, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&letter, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&number, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_displayed_body_role_rescue(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[1].lines[0].role, LiquidBlockRole::Paragraph);

    let intro = body_test_line("p2:l24", 2, 24, "As the court explained:");
    let mut quote = body_test_line(
        "p2:l25",
        2,
        25,
        "We have concluded that one member may act for the Board",
    );
    quote.left = 23.0;
    quote.font_height = 8.7;
    let mut note = body_test_line("p2:l26", 2, 26, "220. 102 F.3d 579, 582.");
    note.segment_block_id = 8;
    note.segment_block_shape = "footnote".to_owned();
    note.segment_block_footnote_like = true;
    note.in_footnote_zone = true;
    let decoded = vec![
        (intro.clone(), Lm2Action::Keep),
        (quote.clone(), Lm2Action::Marginalia),
        (note.clone(), Lm2Action::Marginalia),
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![220];
    let mut blocks = vec![
        paragraph_block(&intro.text),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: quote.text.clone(),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&intro, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&quote, LiquidBlockRole::Marginalia)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![note_ref],
        },
    ];
    assert_eq!(
        apply_final_displayed_body_role_rescue(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Quote);
}

#[test]
fn final_displayed_body_rescue_keeps_exact_stanford_quote_in_source_order() {
    let mut lead = body_test_line("p35:l24", 35, 24, "quorum. As the court explained:");
    lead.page_width = 468.0;
    lead.left = 138.24054;
    lead.bottom = 394.22232;
    lead.top = 405.428;
    lead.font_height = 10.775;
    lead.segment_block_id = 2;
    lead.segment_block_line_index = 21;
    lead.segment_block_line_count = 26;
    let quote_texts = [
        "We have concluded that a single member of the National Mediation Board may act",
        "for the Board pursuant to a validly issued delegation order that is narrowly tailored",
        "to prevent the temporary occurrence of two vacancies from completely disabling",
        "the Board. We believe that our conclusion is compelled by a close reading of the",
    ];
    let quote_lines = quote_texts
        .iter()
        .enumerate()
        .map(|(offset, text)| {
            let line_index = 25 + offset;
            let mut line = body_test_line(&format!("p35:l{line_index}"), 35, line_index, text);
            line.page_width = 468.0;
            line.left = 156.24;
            line.bottom = 380.33545 - offset as f32 * 11.02;
            line.top = line.bottom + 8.5415;
            line.font_height = 9.48;
            line.font_ratio_doc = 0.8713;
            line.segment_block_id = 2;
            line.segment_block_line_index = 22 + offset;
            line.segment_block_line_count = 26;
            line
        })
        .collect::<Vec<_>>();
    let mut note = body_test_line(
        "p35:l29",
        35,
        29,
        "220. 102 F.3d 579, 582 (D.C. Cir. 1996).",
    );
    note.page_width = 468.0;
    note.segment_block_id = 3;
    note.segment_block_line_index = 0;
    note.segment_block_line_count = 2;
    note.segment_block_shape = "table".to_owned();
    note.segment_block_footnote_like = true;
    note.in_footnote_zone = true;
    note.font_height = 9.41;
    note.font_ratio_doc = 0.865;
    let mut tail = body_test_line(
        "p36:l3",
        36,
        3,
        "plain words of the statute and is fully consistent with the legislative history.",
    );
    tail.page_width = 468.0;
    tail.left = 156.24;
    tail.bottom = 659.27545;
    tail.top = 667.8169;
    tail.font_height = 9.48;
    tail.font_ratio_doc = 0.8713;
    tail.segment_block_id = 2;
    tail.segment_block_line_index = 0;
    tail.segment_block_line_count = 19;

    let mut decoded = vec![(lead.clone(), Lm2Action::Keep)];
    decoded.extend(
        quote_lines
            .iter()
            .cloned()
            .map(|line| (line, Lm2Action::Marginalia)),
    );
    decoded.push((note.clone(), Lm2Action::Marginalia));
    decoded.push((tail.clone(), Lm2Action::Keep));
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![220];
    let mut blocks = vec![
        paragraph_block(&lead.text),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: quote_texts.join(" "),
            label: None,
        },
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
        paragraph_block(&tail.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&lead, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: quote_lines
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Marginalia))
                .collect(),
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![note_ref],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&tail, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_displayed_body_role_rescue(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Quote);
    assert_eq!(
        apply_inverted_numbered_note_continuation_reflow(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks[1].role, LiquidBlockRole::Quote);
    assert_eq!(
        blocks[1].text,
        format!("{} {}", quote_texts.join(" "), tail.text)
    );
    assert!(!blocks[2].text.contains("We have concluded"));
}

#[test]
fn final_furniture_prefix_suppression_preserves_first_body_row() {
    let mut slug = body_test_line(
        "p0:l0",
        0,
        0,
        "DEACON AND LITMAN IN PRINTER DELETE) T4/26/2026 1:05 PM",
    );
    slug.segment_block_shape = "furniture".to_owned();
    let mut header = body_test_line("p1:l1", 1, 1, "2026] LEGALISTIC NONCOMPLIANCE 1459");
    header.role_hint = Some(LiquidBlockRole::Noise);
    let body = body_test_line(
        "p1:l2",
        1,
        2,
        "“the United States’ decision on the[] [detainees’] long term disposition.”100",
    );
    let mut body = body;
    body.text = format!(
        "{}{CALLOUT_START}100{CALLOUT_END}",
        body.text.trim_end_matches("100")
    );
    body.role_hint = Some(LiquidBlockRole::Noise);
    assert!(looks_like_running_header(&normalize_text(&body.text)));
    assert!(!lm2_final_hard_furniture_line(&body));
    let decoded = vec![
        (slug.clone(), Lm2Action::HideNoise),
        (header.clone(), Lm2Action::HideNoise),
        (body.clone(), Lm2Action::HideNoise),
    ];
    let mut blocks = vec![
        paragraph_block(&slug.text),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: format!("{} {}", header.text, body.text),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&slug, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&header, LiquidBlockRole::Noise),
                line_ref(&body, LiquidBlockRole::Noise),
            ],
        },
    ];
    assert_eq!(
        apply_final_mixed_furniture_prefix_suppression(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks[0].role, LiquidBlockRole::Noise);
    assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    assert_eq!(blocks[1].text, body.text);
    assert_eq!(sources[1].lines[0].role, LiquidBlockRole::Paragraph);
    assert_eq!(sources[1].lines[0].id.as_deref(), Some("p1:l2"));
}

#[test]
fn final_split_then_coalescer_restores_wrapped_outdent_and_hyphen() {
    let mut same = body_test_line("p49:l4", 49, 4, "echoed the same");
    same.left = 24.1;
    let ideas = body_test_line("p49:l5", 49, 5, "ideas. The White House responded.");
    let mut author = body_test_line("p75:l14", 75, 14, "state-conferred author\u{0002}");
    let mut ity = body_test_line("p75:l15", 75, 15, "ity and cross-jurisdictional");
    let mut assignment = body_test_line("p75:l16", 75, 16, "assignment disputes remain difficult.");
    author.segment_block_id = 8;
    ity.segment_block_id = 8;
    assignment.segment_block_id = 9;
    author.page_width = 612.0;
    ity.page_width = 612.0;
    assignment.page_width = 612.0;
    author.page_height = 792.0;
    ity.page_height = 792.0;
    assignment.page_height = 792.0;
    author.left = 94.1;
    ity.left = 77.9;
    assignment.left = 77.9;
    author.bottom = 497.0;
    author.top = 498.0;
    ity.bottom = 462.35;
    ity.top = 523.6;
    assignment.bottom = 438.0;
    assignment.top = 462.0;
    let decoded = vec![
        (same.clone(), Lm2Action::Keep),
        (ideas.clone(), Lm2Action::Keep),
        (author.clone(), Lm2Action::Keep),
        (ity.clone(), Lm2Action::Keep),
        (assignment.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&same.text),
        paragraph_block(&ideas.text),
        paragraph_block("state-conferred author"),
        paragraph_block(&ity.text),
        paragraph_block(&assignment.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&same, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&ideas, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&author, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&ity, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 4,
            lines: vec![line_ref(&assignment, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_source_backed_paragraph_splits(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        3
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains("same ideas"));
    assert_eq!(
        blocks[1].text,
        "state-conferred authority and cross-jurisdictional assignment disputes remain difficult."
    );
    assert_eq!(
        apply_final_source_backed_paragraph_splits(&mut blocks, &mut sources, &decoded),
        0
    );
}

#[test]
fn final_body_coalescer_accepts_numeric_inline_marker_outdent() {
    let mut previous = body_test_line(
        "p20:l19",
        20,
        19,
        "92 The Department of Homeland Security has created",
    );
    let mut current = body_test_line(
        "p20:l20",
        20,
        20,
        "a network of intergovernmental partnerships.",
    );
    previous.left = 30.0;
    current.left = 13.0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("The Department of Homeland Security has created"),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        "The Department of Homeland Security has created a network of intergovernmental partnerships."
    );
}

#[test]
fn final_body_coalescer_moves_a_leading_callout_back_into_same_segment_prose() {
    let mut previous = body_test_line("p4:l3", 4, 3, "for heightened scrutiny to apply.");
    let mut current = body_test_line(
        "p4:l4",
        4,
        4,
        "14 So long as people remain free to associate.",
    );
    previous.page_width = 486.0;
    current.page_width = 486.0;
    previous.page_height = 720.0;
    current.page_height = 720.0;
    previous.left = 88.344;
    current.left = 88.344;
    previous.right = 113.6;
    previous.last_visual_right = 113.6;
    current.first_visual_left = 113.7;
    current.right = 412.4;
    previous.bottom = 600.067;
    previous.top = 614.538;
    current.bottom = 588.067;
    current.top = 614.538;
    previous.font_height = 10.54;
    current.font_height = 9.124;
    previous.segment_block_id = 3;
    current.segment_block_id = 3;
    previous.segment_block_line_index = 1;
    current.segment_block_line_index = 0;
    previous.segment_block_line_count = 2;
    current.segment_block_line_count = 2;
    assert!(lm2_final_paragraph_boundary(&previous, &current));
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].text,
        "for heightened scrutiny to apply. 14 So long as people remain free to associate."
    );
}

#[test]
fn final_body_coalescer_preserves_a_new_paragraph_with_a_leading_callout() {
    let mut previous = body_test_line("p4:l3", 4, 3, "The prior paragraph ends.");
    let mut current = body_test_line("p4:l4", 4, 4, "14 A genuinely new paragraph begins.");
    previous.segment_block_id = 3;
    current.segment_block_id = 3;
    previous.segment_block_line_index = 1;
    current.segment_block_line_index = 2;
    previous.bottom = 600.0;
    previous.top = 614.0;
    current.bottom = 582.0;
    current.top = 596.0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 2);
}

#[test]
fn paragraph_boundary_uses_the_first_row_of_a_multirow_source_item() {
    let mut previous = body_test_line(
        "p34:l11",
        34,
        11,
        "The prior paragraph ends with its members.",
    );
    let mut current = body_test_line(
        "p34:l12",
        34,
        12,
        "The Supreme Court's opinion starts a new paragraph.\u{0002}The wrapped row returns to the margin.",
    );
    previous.page_width = 486.0;
    current.page_width = 486.0;
    previous.left = 88.3;
    previous.right = 412.3;
    current.left = 88.3;
    current.right = 412.3;
    current.first_visual_left = 106.3;
    previous.bottom = 480.1;
    previous.top = 506.5;
    current.bottom = 456.1;
    current.top = 482.5;

    assert!(paragraph_boundary(&previous, &current));
}

#[test]
fn final_body_coalescer_restores_exact_yale_role_boundary_wraps() {
    let mut homeland = body_test_line(
        "p20:l19",
        20,
        19,
        "92 The Department of Homeland Security has created",
    );
    let mut network = body_test_line(
        "p20:l20",
        20,
        20,
        "a network of intergovernmental partnerships.",
    );
    let mut president = body_test_line(
        "p44:l18",
        44,
        18,
        "227 The Presidential Records Act permits the President",
    );
    let mut care = body_test_line(
        "p44:l19",
        44,
        19,
        "to take care that the records remain available.",
    );
    for line in [&mut homeland, &mut network, &mut president, &mut care] {
        line.page_width = 486.0;
        line.page_height = 720.0;
        line.segment_block_shape = "body".to_owned();
    }
    homeland.left = 136.14003;
    homeland.bottom = 434.20145;
    homeland.top = 445.52225;
    network.left = 57.95987;
    network.bottom = 421.24176;
    network.top = 432.1998;
    president.left = 100.50003;
    president.bottom = 433.24146;
    president.top = 444.56226;
    care.left = 57.96021;
    care.bottom = 420.22137;
    care.top = 431.1794;
    homeland.text =
        format!("{CALLOUT_START}92{CALLOUT_END} The Department of Homeland Security has created");
    president.text = format!(
        "{CALLOUT_START}227{CALLOUT_END} The Presidential Records Act permits the President"
    );
    let decoded = vec![
        (homeland.clone(), Lm2Action::Keep),
        (network.clone(), Lm2Action::Keep),
        (president.clone(), Lm2Action::Keep),
        (care.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("The Department of Homeland Security has created"),
        paragraph_block(&network.text),
        paragraph_block("The Presidential Records Act permits the President"),
        paragraph_block(&care.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&homeland, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&network, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&president, LiquidBlockRole::Heading)],
        },
        LiquidBlockSourceLines {
            block_index: 3,
            lines: vec![line_ref(&care, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        2
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        "The Department of Homeland Security has created a network of intergovernmental partnerships."
    );
    assert_eq!(
        blocks[1].text,
        "The Presidential Records Act permits the President to take care that the records remain available."
    );
}

#[test]
fn final_body_coalescer_restores_exact_penn_quote_role_boundary() {
    let mut competent = body_test_line(
        "p16:l15",
        16,
        15,
        "designed around the premise that parties represented by competent counsel",
    );
    let mut know = body_test_line(
        "p16:l16",
        16,
        16,
        "know what is best for them, and are responsible for advancing the facts and",
    );
    let mut argument = body_test_line("p16:l17", 16, 17, "argument entitling them to relief.");
    for (offset, line) in [&mut competent, &mut know, &mut argument]
        .into_iter()
        .enumerate()
    {
        line.page_width = 468.0;
        line.page_height = 720.0;
        line.left = 87.3946;
        line.bottom = 443.6673 - offset as f32 * 12.963;
        line.top = line.bottom + 11.1686;
        line.segment_block_id = 2;
        line.segment_block_line_index = 7 + offset;
        line.segment_block_line_count = 18;
        line.segment_block_shape = "body".to_owned();
    }
    let decoded = vec![
        (competent.clone(), Lm2Action::Keep),
        (know.clone(), Lm2Action::Keep),
        (argument.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&competent.text),
        paragraph_block(&format!("{} {}", know.text, argument.text)),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&competent, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                line_ref(&know, LiquidBlockRole::Marginalia),
                line_ref(&argument, LiquidBlockRole::Marginalia),
            ],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0]
            .text
            .contains("competent counsel know what is best")
    );
}

#[test]
fn final_body_edges_accept_inline_callout_fragments_and_terminal_marker_rows() {
    let mut inline_left = body_test_line("p0:l18", 0, 18, "v. Abbasi and Egbert v. Boule,");
    inline_left.segment_block_shape = "footnote".to_owned();
    inline_left.right = 42.0;
    inline_left.bottom = 50.0;
    inline_left.top = 60.0;
    let mut inline_right = body_test_line(
        "p0:l19",
        0,
        19,
        "77 the Court converted that narrowness into a presumption",
    );
    inline_right.segment_block_id = 8;
    inline_right.segment_block_shape = "mixed".to_owned();
    inline_right.left = 42.0;
    inline_right.bottom = 50.0;
    inline_right.top = 60.0;
    let decoded = vec![
        (inline_left.clone(), Lm2Action::Keep),
        (inline_right.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("v. Abbasi and Egbert v. Boule,"),
        paragraph_block("the Court converted that narrowness into a presumption"),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&inline_left, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&inline_right, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);

    let prior = body_test_line("p1:l5", 1, 5, "pursuant to federal priorities,");
    let mut marker = body_test_line("p1:l6", 1, 6, "80");
    marker.segment_block_shape = "furniture".to_owned();
    let next = body_test_line("p1:l7", 1, 7, "while the remedial scheme continues");
    let decoded = vec![
        (prior.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::HideNoise),
        (next.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block("pursuant to federal priorities,\u{E000}80\u{E001}"),
        paragraph_block(&next.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                line_ref(&prior, LiquidBlockRole::Paragraph),
                line_ref(&marker, LiquidBlockRole::Noise),
            ],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&next, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
}

#[test]
fn cross_page_coalescer_uses_first_body_row_after_top_notes() {
    let previous = body_test_line("p0:l30", 0, 30, "authority and then");
    let mut note = body_test_line("p1:l2", 1, 2, "80. A top-of-page note.");
    note.segment_block_shape = "footnote".to_owned();
    note.segment_block_footnote_like = true;
    note.in_footnote_zone = true;
    let current = body_test_line("p1:l7", 1, 7, "proceed through the corresponding channel.");
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (note.clone(), Lm2Action::Marginalia),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut note_ref = line_ref(&note, LiquidBlockRole::Marginalia);
    note_ref.note_markers = vec![80];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note.text.clone(),
            label: None,
        },
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![note_ref],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];
    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains("proceed through"));
}

#[test]
fn final_body_coalescer_crosses_a_standalone_same_row_callout() {
    let mut previous = body_test_line(
        "p0:l10",
        0,
        10,
        "building maintenance crews.\u{E000}329\u{E001}",
    );
    let mut marker = body_test_line("p0:l11", 0, 11, "329");
    let mut current = body_test_line("p0:l12", 0, 12, "It does not require");
    previous.page_width = 612.0;
    marker.page_width = 612.0;
    current.page_width = 612.0;
    previous.page_height = 792.0;
    marker.page_height = 792.0;
    current.page_height = 792.0;
    previous.left = 90.0;
    previous.right = 300.0;
    previous.first_visual_left = 90.0;
    previous.last_visual_right = 300.0;
    previous.bottom = 400.0;
    previous.top = 414.0;
    marker.left = 300.5;
    marker.right = 310.0;
    marker.first_visual_left = 300.5;
    marker.last_visual_right = 310.0;
    marker.bottom = 405.0;
    marker.top = 413.0;
    current.left = 312.0;
    current.right = 410.0;
    current.first_visual_left = 312.0;
    current.last_visual_right = 410.0;
    current.bottom = 400.0;
    current.top = 414.0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::HideNoise),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: marker.text.clone(),
            label: None,
        },
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&marker, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].text.contains("crews."));
    assert!(blocks[0].text.contains("It does not require"));
}

#[test]
fn final_body_coalescer_preserves_an_indented_paragraph_after_a_standalone_callout() {
    let mut previous = body_test_line("p34:l19", 34, 19, "be coerced with the threat of eviction.");
    let mut marker = body_test_line("p34:l20", 34, 20, "225");
    let mut current = body_test_line(
        "p34:l21",
        34,
        21,
        "Although some courts have identified material benefits.",
    );
    for line in [&mut previous, &mut marker, &mut current] {
        line.page_width = 486.0;
        line.page_height = 720.0;
    }
    previous.left = 88.3;
    previous.right = 246.0;
    previous.first_visual_left = 88.3;
    previous.last_visual_right = 246.0;
    previous.bottom = 384.1;
    previous.top = 398.5;
    marker.left = 246.0;
    marker.right = 255.4;
    marker.first_visual_left = 246.0;
    marker.last_visual_right = 255.4;
    marker.bottom = 389.0;
    marker.top = 397.9;
    current.left = 88.1;
    current.right = 412.2;
    current.first_visual_left = 106.0;
    current.last_visual_right = 412.2;
    current.bottom = 360.1;
    current.top = 386.5;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (marker.clone(), Lm2Action::HideNoise),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        LiquidBlock {
            role: LiquidBlockRole::Noise,
            text: marker.text.clone(),
            label: None,
        },
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&marker, LiquidBlockRole::Noise)],
        },
        LiquidBlockSourceLines {
            block_index: 2,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        0
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].text, previous.text);
    assert_eq!(blocks[2].text, current.text);
}

#[test]
fn final_body_coalescer_crosses_a_dropped_same_row_callout() {
    let mut previous = body_test_line("p0:l10", 0, 10, "religious organizations,340");
    let mut marker = body_test_line("p0:l11", 0, 11, "340");
    let mut current = body_test_line("p0:l12", 0, 12, "the Church would have a");
    for line in [&mut previous, &mut marker, &mut current] {
        line.page_width = 612.0;
        line.page_height = 792.0;
    }
    previous.left = 90.0;
    previous.right = 288.0;
    previous.first_visual_left = 90.0;
    previous.last_visual_right = 288.0;
    previous.bottom = 400.0;
    previous.top = 414.0;
    marker.left = 288.4;
    marker.right = 298.0;
    marker.first_visual_left = 288.4;
    marker.last_visual_right = 298.0;
    marker.bottom = 405.0;
    marker.top = 413.0;
    current.left = 301.0;
    current.right = 412.0;
    current.first_visual_left = 301.0;
    current.last_visual_right = 412.0;
    current.bottom = 400.0;
    current.top = 414.0;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (marker, Lm2Action::HideNoise),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        paragraph_block(&current.text),
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Paragraph)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.contains("organizations,"));
    assert!(blocks[0].text.contains("the Church would have"));
}

#[test]
fn final_body_coalescer_restores_centered_false_heading_inside_body_segment() {
    let mut previous = body_test_line(
        "p6:l5",
        6,
        5,
        "A sister of presidential administration is what",
    );
    let mut current = body_test_line(
        "p6:l6",
        6,
        6,
        "Professors Timothy Meyer and Ganesh Sitaraman have recently termed",
    );
    previous.segment_block_line_index = 4;
    current.segment_block_line_index = 5;
    previous.centered = true;
    current.centered = true;
    let decoded = vec![
        (previous.clone(), Lm2Action::Keep),
        (current.clone(), Lm2Action::Keep),
    ];
    let mut blocks = vec![
        paragraph_block(&previous.text),
        LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: current.text.clone(),
            label: None,
        },
    ];
    let mut sources = vec![
        LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![line_ref(&previous, LiquidBlockRole::Paragraph)],
        },
        LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line_ref(&current, LiquidBlockRole::Heading)],
        },
    ];

    assert_eq!(
        apply_final_body_source_order_and_coalescing(&mut blocks, &mut sources, &decoded),
        1
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].text.contains("what Professors Timothy Meyer"));
}
