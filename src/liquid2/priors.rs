//! Decoder priors and transition score maps for the fallback tier.

use super::*;

pub(super) fn decoder_line_prior(
    runtime: &Lm2Runtime,
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> f64 {
    let mut score = 0.0;
    if runtime.marker_decoder_prior {
        score += marker_continuity_decoder_prior(line, action);
    }
    if runtime.small_font_decoder_prior {
        score += small_font_lower_page_decoder_prior(line, action);
    }
    score
}

pub(super) fn decoder_transition_prior(
    runtime: &Lm2Runtime,
    _previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    let mut score = 0.0;
    if runtime.small_font_sequence_prior {
        score += small_font_sequence_continuation_prior(line, previous, current);
    }
    score
}

pub(super) fn small_font_sequence_continuation_prior(
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    if previous != Lm2Action::Marginalia || current != Lm2Action::Marginalia {
        return 0.0;
    }
    if !small_font_lower_page_decoder_prior_eligible(line) {
        return 0.0;
    }
    2.1
}

pub(super) fn marker_continuity_decoder_prior(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> f64 {
    if !marker_continuity_decoder_prior_eligible(line) {
        return 0.0;
    }
    match action {
        Lm2Action::Marginalia => 1.6,
        Lm2Action::Keep => -0.2,
        Lm2Action::HideNoise => 0.0,
    }
}

pub(super) fn marker_continuity_decoder_prior_eligible(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if looks_like_toc_entry(&lower) || looks_like_running_header(&lower) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    if line.doc_repeated_edge_text && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge)
    {
        return false;
    }

    let marker_signal = line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page;
    if !marker_signal {
        return false;
    }

    let y_bottom = line.bottom / line.page_height.max(1.0);
    let small_font = line.font_ratio_page < 0.90 || line.font_ratio_doc < 0.90;
    looks_like_note_start(&line.text)
        || has_legal_note_cue(&lower)
        || line.below_footnote_divider
        || y_bottom < 0.26
        || small_font && y_bottom < 0.42
}

pub(super) fn small_font_lower_page_decoder_prior(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> f64 {
    if !small_font_lower_page_decoder_prior_eligible(line) {
        return 0.0;
    }
    match action {
        Lm2Action::Marginalia => 2.25,
        Lm2Action::Keep => -0.20,
        Lm2Action::HideNoise => 0.0,
    }
}

pub(super) fn small_font_lower_page_decoder_prior_eligible(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if looks_like_toc_entry(&lower) || looks_like_running_header(&lower) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    if line.doc_repeated_edge_text && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge)
    {
        return false;
    }
    if looks_like_small_font_page_furniture(&lower) {
        return false;
    }
    if word_count(&lower) <= 8
        && uppercase_ratio(&line.text) >= 0.62
        && !looks_like_note_start(&line.text)
    {
        return false;
    }

    let y_bottom = line.bottom / line.page_height.max(1.0);
    y_bottom < 0.34
        && (line.font_ratio_doc < 0.90 || line.font_ratio_page < 0.84)
        && small_font_note_evidence(line, &lower)
}

pub(super) fn small_font_note_evidence(line: &DeepLiquidSourceLine, lower: &str) -> bool {
    looks_like_note_start(&line.text)
        || has_legal_note_cue(lower)
        || line.below_footnote_divider
        || line.page_has_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page
}

pub(super) fn hard_marginalia_anchor(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    small_font_note_evidence(line, &lower) || looks_like_marginalia_note_block_start(&line.text)
}

pub(super) fn apply_anchored_marginalia_flow_guard(
    lines: &[DeepLiquidSourceLine],
    path: &mut [Lm2Action],
) {
    let mut anchored_run_active = false;
    let mut unanchored_after_anchor = 0usize;
    for (line, action) in lines.iter().zip(path.iter_mut()) {
        if *action != Lm2Action::Marginalia {
            anchored_run_active = false;
            unanchored_after_anchor = 0;
            continue;
        }

        if hard_marginalia_anchor(line) {
            anchored_run_active = true;
            unanchored_after_anchor = 0;
            continue;
        }

        if !anchored_run_active {
            *action = Lm2Action::Keep;
            continue;
        }

        unanchored_after_anchor += 1;
        if unanchored_after_anchor > 2 {
            *action = Lm2Action::Keep;
        }
    }
}

pub(super) fn looks_like_small_font_page_furniture(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("[vol.") || trimmed.starts_with("vol.") {
        return true;
    }
    if trimmed.starts_with("page ")
        && trimmed
            .split_whitespace()
            .nth(1)
            .is_some_and(|token| token.chars().all(|ch| ch.is_ascii_digit()))
    {
        return true;
    }
    if trimmed.starts_with("copyright ")
        || trimmed.contains("all rights reserved")
        || trimmed.contains("published by ")
        || trimmed.contains("brought to you for free and open access")
        || trimmed.contains("accepted for inclusion")
        || trimmed.contains("law archive of scholarship")
        || trimmed.contains("scholar commons")
        || trimmed.contains("scholarcommons")
        || trimmed.contains("for more information, please contact")
        || trimmed.contains("journal of legal studies")
        || trimmed.contains("university press")
        || trimmed.contains("footnote continued")
        || trimmed.contains("received:") && trimmed.contains("revised:")
    {
        return true;
    }
    if trimmed.contains('@') || trimmed.contains("http://") || trimmed.contains("https://") {
        return true;
    }
    false
}

pub(super) fn final_lm2_action(line: &DeepLiquidSourceLine, action: Lm2Action) -> Lm2Action {
    if looks_like_production_slug_boilerplate(&line.text) {
        Lm2Action::HideNoise
    } else if synthetic_ocr_body_preservation_candidate(line, action) {
        Lm2Action::Keep
    } else {
        action
    }
}

pub(super) fn synthetic_ocr_body_preservation_candidate(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> bool {
    if !line.synthetic_text_geometry
        || action != Lm2Action::HideNoise
        || line.below_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || matches!(
            line.role_hint,
            Some(
                LiquidBlockRole::Marginalia
                    | LiquidBlockRole::Footnote
                    | LiquidBlockRole::Noise
                    | LiquidBlockRole::Header
                    | LiquidBlockRole::Footer
                    | LiquidBlockRole::Metadata
                    | LiquidBlockRole::Caption
                    | LiquidBlockRole::Table
                    | LiquidBlockRole::Contents
            )
        )
    {
        return false;
    }
    let lower = normalize_text(&line.text);
    if lower.trim().is_empty()
        || looks_like_note_start(&line.text)
        || has_legal_note_cue(&lower)
        || looks_like_toc_entry(&lower)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
    {
        return false;
    }
    let words = word_count(&lower);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let width = (line.right - line.left).max(0.0) / line.page_width.max(1.0);
    // Document-wide font ratios are not meaningful when most pages use
    // synthetic OCR boxes and a few retain native PDF geometry. Page-relative
    // size remains stable and is the conservative signal here.
    words >= 8
        && (0.10..=0.92).contains(&y_bottom)
        && width >= 0.28
        && line.font_ratio_page >= 0.90
        && uppercase_ratio(&line.text) < 0.72
}

pub(super) fn apply_synthetic_ocr_body_preservation(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded {
        if synthetic_ocr_body_preservation_candidate(line, *action) {
            *action = Lm2Action::Keep;
        }
    }
}

/// Recover an unlabeled law-review abstract that the model routed to
/// marginalia because journals often typeset abstracts in a smaller font.
///
/// Scope is deliberately narrow: a long prose run on the first two pages,
/// before `INTRODUCTION`, with substantial marginalia participation. A
/// dagger/asterisk author note ends the run so acknowledgements stay notes.
pub(super) fn apply_front_matter_abstract_recovery(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    let stop = decoded
        .iter()
        .position(|(line, _)| normalize_text(&line.text) == "introduction")
        .unwrap_or(decoded.len());
    let mut run = Vec::new();
    let mut blocked_page = None;

    for index in 0..stop {
        let (line, action) = &decoded[index];
        if blocked_page == Some(line.page_index) {
            continue;
        }
        blocked_page = None;
        let author_or_note_start = lm2_front_abstract_note_start(line);
        let starts_new_page = run
            .first()
            .is_some_and(|first: &usize| decoded[*first].0.page_index != line.page_index);
        let eligible = line.page_index <= 1
            && matches!(*action, Lm2Action::Keep | Lm2Action::Marginalia)
            && !line.below_footnote_divider
            && !lm2_front_abstract_hard_break(line)
            && (run.is_empty() && word_count(&line.text) >= 5
                || !run.is_empty() && word_count(&line.text) >= 1);

        if starts_new_page || !eligible {
            if lm2_front_abstract_run_is_plausible(decoded, &run) {
                lm2_mark_front_abstract_run(decoded, &run);
                return;
            }
            run.clear();
        }
        if author_or_note_start {
            // A short starred/daggered name commonly sits immediately above
            // the abstract. A longer marked line is the author note itself.
            if word_count(&line.text) > 6 {
                blocked_page = Some(line.page_index);
            }
            continue;
        }
        if eligible {
            run.push(index);
        }
    }

    if lm2_front_abstract_run_is_plausible(decoded, &run) {
        lm2_mark_front_abstract_run(decoded, &run);
    }
}

pub(super) fn lm2_front_abstract_note_start(line: &DeepLiquidSourceLine) -> bool {
    let text = line.text.trim_start();
    text.starts_with('*')
        || text.starts_with('∗')
        || text.starts_with('†')
        || text.starts_with('‡')
        || looks_like_marginalia_note_block_start(text)
        || looks_like_note_start(text)
}

pub(super) fn lm2_front_abstract_hard_break(line: &DeepLiquidSourceLine) -> bool {
    let text = line.text.trim_start();
    lm2_front_abstract_note_start(line)
        || lm2_toc_dotleader_line(text)
        || looks_like_production_slug_boilerplate(text)
        || matches!(
            line.role_hint,
            Some(
                LiquidBlockRole::Title
                    | LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Contents
                    | LiquidBlockRole::Table
                    | LiquidBlockRole::Caption
            )
        )
}

pub(super) fn lm2_front_abstract_run_is_plausible(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    run: &[usize],
) -> bool {
    if run.len() < 7 {
        return false;
    }
    let words = run
        .iter()
        .map(|index| word_count(&decoded[*index].0.text))
        .sum::<usize>();
    let marginalia = run
        .iter()
        .filter(|index| decoded[**index].1 == Lm2Action::Marginalia)
        .count();
    words >= 90 && marginalia >= 3 && marginalia * 10 >= run.len() * 3
}

pub(super) fn lm2_mark_front_abstract_run(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
    run: &[usize],
) {
    for index in run {
        decoded[*index].1 = Lm2Action::Keep;
        decoded[*index].0.role_hint = Some(LiquidBlockRole::Abstract);
    }
}

pub(super) fn start_cost(role_hint: Option<LiquidBlockRole>, action: Lm2Action) -> f64 {
    match (role_hint, action) {
        (Some(role), action) if role_action(role) == action => 0.6,
        (_, Lm2Action::HideNoise) => 0.15,
        _ => 0.0,
    }
}

pub(super) fn transition_score(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    if previous == current {
        return match current {
            Lm2Action::Keep => 0.52,
            Lm2Action::Marginalia => 0.82,
            Lm2Action::HideNoise => 0.62,
        };
    }
    let gap = vertical_gap(previous_line, line);
    let font_drop = line.font_ratio_page < 0.88 || line.font_ratio_doc < 0.88;
    let below_divider = line.below_footnote_divider || previous_line.below_footnote_divider;
    match (previous, current) {
        (Lm2Action::Keep, Lm2Action::Marginalia) if below_divider || font_drop => 0.20,
        (Lm2Action::Marginalia, Lm2Action::Keep) if !below_divider && gap > 0.035 => -0.25,
        (Lm2Action::HideNoise, Lm2Action::Keep) if gap > 0.025 => 0.05,
        (Lm2Action::Keep, Lm2Action::HideNoise) if is_edge_line(line) => -0.05,
        (Lm2Action::HideNoise, Lm2Action::Marginalia) if below_divider => 0.0,
        (Lm2Action::Marginalia, Lm2Action::HideNoise) if is_edge_line(line) => -0.15,
        (Lm2Action::Keep, Lm2Action::Marginalia) => -1.05,
        (Lm2Action::Marginalia, Lm2Action::Keep) => -1.25,
        _ => -0.58,
    }
}

pub(super) fn decoder_start_correction(
    runtime: &Lm2Runtime,
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> f64 {
    let Some(weights) = runtime.decoder_weights() else {
        return 0.0;
    };
    let action_name = action.as_str();
    let mut score = *weights
        .get(&format!("start_arc:{action_name}"))
        .unwrap_or(&0.0);
    for (name, value) in start_feature_map(line) {
        score += weights
            .get(&format!("start_feature:{action_name}:{name}"))
            .copied()
            .unwrap_or(0.0)
            * value;
    }
    score
}

pub(super) fn decoder_transition_correction(
    runtime: &Lm2Runtime,
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    let Some(weights) = runtime.decoder_weights() else {
        return 0.0;
    };
    let arc = format!("{}->{}", previous.as_str(), current.as_str());
    let mut score = *weights
        .get(&format!("transition_arc:{arc}"))
        .unwrap_or(&0.0);
    for (name, value) in transition_feature_map(previous_line, line) {
        score += weights
            .get(&format!("transition_feature:{arc}:{name}"))
            .copied()
            .unwrap_or(0.0)
            * value;
    }
    score
}

pub(super) fn apply_layout_priors(line: &DeepLiquidSourceLine, scores: &mut [f64; 3]) {
    let lower = normalize_text(&line.text);
    let words = word_count(&lower);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let role_action = line.role_hint.map(role_action);
    if let Some(action) = role_action {
        let note_zone =
            line.below_footnote_divider || y_bottom < 0.26 || looks_like_note_start(&line.text);
        scores[action.index()] += match action {
            Lm2Action::Keep => 0.8,
            Lm2Action::Marginalia if note_zone => 0.65,
            Lm2Action::Marginalia => 0.0,
            Lm2Action::HideNoise => 0.9,
        };
    }
    if line.below_footnote_divider {
        scores[Lm2Action::Marginalia.index()] += 2.1;
        scores[Lm2Action::Keep.index()] -= 0.8;
    }
    if line.page_has_footnote_divider && line.bottom / line.page_height.max(1.0) < 0.32 {
        scores[Lm2Action::Marginalia.index()] += 0.75;
    }
    if (line.font_ratio_page < 0.84 || line.font_ratio_doc < 0.84)
        && (line.below_footnote_divider
            || y_bottom < 0.36
            || looks_like_note_start(&line.text)
            || has_legal_note_cue(&lower))
    {
        scores[Lm2Action::Marginalia.index()] += 0.72;
    }
    if !line.below_footnote_divider && y_bottom > 0.22 && !looks_like_note_start(&line.text) {
        scores[Lm2Action::Keep.index()] += 1.5;
        scores[Lm2Action::Marginalia.index()] -= 1.5;
    }
    if line.centered && (line.font_ratio_page > 1.08 || words <= 12) {
        scores[Lm2Action::Keep.index()] += 0.55;
    }
    if line.bold && words <= 14 && line.font_ratio_page >= 0.98 {
        scores[Lm2Action::Keep.index()] += 0.45;
    }
    if looks_like_note_start(&line.text) || has_legal_note_cue(&lower) {
        scores[Lm2Action::Marginalia.index()] += 0.9;
    }
    if looks_like_toc_entry(&lower) || is_edge_line(line) && words <= 8 {
        scores[Lm2Action::HideNoise.index()] += 0.9;
    }
    if lower.len() <= 4 && lower.chars().all(|ch| ch.is_ascii_digit()) {
        scores[Lm2Action::HideNoise.index()] += 6.0;
        scores[Lm2Action::Keep.index()] -= 1.0;
        scores[Lm2Action::Marginalia.index()] -= 1.0;
    }
    if looks_like_running_header(&lower) {
        scores[Lm2Action::HideNoise.index()] += 10.0;
        scores[Lm2Action::Keep.index()] -= 5.0;
        scores[Lm2Action::Marginalia.index()] -= 1.0;
    }
}

pub(super) fn apply_pp_priors(
    runtime: &Lm2Runtime,
    line: &DeepLiquidSourceLine,
    scores: &mut [f64; 3],
) {
    if runtime.pp_footnote_region_membership {
        return;
    }
    let Some(prior) = runtime.pp_prior_for_line(line) else {
        return;
    };
    if prior.role == "footnote" && prior.score >= 0.80 {
        scores[Lm2Action::Marginalia.index()] += 3.8;
        scores[Lm2Action::Keep.index()] -= 1.2;
        scores[Lm2Action::HideNoise.index()] -= 0.8;
    } else if prior.role == "table" && prior.score >= 0.70 {
        scores[Lm2Action::HideNoise.index()] += 3.2;
        scores[Lm2Action::Keep.index()] -= 0.7;
        scores[Lm2Action::Marginalia.index()] -= 0.7;
    } else if prior.label == "number" && prior.score >= 0.80 {
        scores[Lm2Action::HideNoise.index()] += 3.5;
        scores[Lm2Action::Keep.index()] -= 0.8;
        scores[Lm2Action::Marginalia.index()] -= 0.8;
    }
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn action_scores_map(scores: [f64; 3]) -> BTreeMap<String, f64> {
    ACTIONS
        .iter()
        .map(|action| (action.as_str().to_owned(), scores[action.index()]))
        .collect()
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn start_scores_map(
    role_hint: Option<LiquidBlockRole>,
    scale: f64,
) -> BTreeMap<String, f64> {
    ACTIONS
        .iter()
        .map(|action| {
            (
                action.as_str().to_owned(),
                scale * start_cost(role_hint, *action),
            )
        })
        .collect()
}

pub(super) fn start_feature_map(line: &DeepLiquidSourceLine) -> BTreeMap<String, f64> {
    let mut features = BTreeMap::new();
    features.insert("bias".to_owned(), 1.0);
    features.insert("first_action_hide_noise".to_owned(), 1.0);
    features.insert(
        "doc_footnote_state".to_owned(),
        bool_as_f64(line.doc_footnote_state),
    );
    features.insert(
        "doc_footnote_continuation".to_owned(),
        bool_as_f64(line.doc_footnote_continuation),
    );
    features.insert(
        "doc_footnote_continuation_no_divider".to_owned(),
        bool_as_f64(line.doc_footnote_continuation && !line.page_has_footnote_divider),
    );
    features.insert(
        "doc_footnote_continuation_small_font".to_owned(),
        bool_as_f64(
            line.doc_footnote_continuation
                && (line.font_ratio_page < 0.94 || line.font_ratio_doc < 0.94),
        ),
    );
    features.insert(
        "doc_note_marker_mid_sequence_page".to_owned(),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
    );
    features.insert(
        "doc_note_marker_follows_previous_page".to_owned(),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
    );
    features.insert(
        "doc_note_marker_first_on_page".to_owned(),
        bool_as_f64(line.doc_note_marker_first_on_page),
    );
    features.insert(
        "doc_note_marker_present".to_owned(),
        bool_as_f64(line.doc_note_marker > 0),
    );
    if let Some(role) = line.role_hint {
        features.insert(
            format!("role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    features
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn transition_scores_map(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    scale: f64,
) -> BTreeMap<String, f64> {
    let mut scores = BTreeMap::new();
    for previous in ACTIONS {
        for current in ACTIONS {
            scores.insert(
                format!("{}->{}", previous.as_str(), current.as_str()),
                scale * transition_score(previous_line, line, previous, current),
            );
        }
    }
    scores
}

pub(super) fn transition_feature_map(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> BTreeMap<String, f64> {
    let mut features = BTreeMap::new();
    let gap = vertical_gap(previous_line, line) as f64;
    let font_drop = line.font_ratio_page < 0.88 || line.font_ratio_doc < 0.88;
    let below_divider_pair = line.below_footnote_divider || previous_line.below_footnote_divider;
    let current_edge_line = is_edge_line(line);
    features.insert("bias".to_owned(), 1.0);
    features.insert("vertical_gap".to_owned(), gap);
    features.insert("font_drop".to_owned(), bool_as_f64(font_drop));
    features.insert(
        "below_divider_pair".to_owned(),
        bool_as_f64(below_divider_pair),
    );
    features.insert(
        "current_edge_line".to_owned(),
        bool_as_f64(current_edge_line),
    );
    features.insert(
        "previous_edge_line".to_owned(),
        bool_as_f64(is_edge_line(previous_line)),
    );
    features.insert(
        "same_page".to_owned(),
        bool_as_f64(previous_line.page_index == line.page_index),
    );
    features.insert(
        "line_below_footnote_divider".to_owned(),
        bool_as_f64(line.below_footnote_divider),
    );
    features.insert(
        "previous_below_footnote_divider".to_owned(),
        bool_as_f64(previous_line.below_footnote_divider),
    );
    features.insert(
        "current_doc_footnote_state".to_owned(),
        bool_as_f64(line.doc_footnote_state),
    );
    features.insert(
        "current_doc_footnote_continuation".to_owned(),
        bool_as_f64(line.doc_footnote_continuation),
    );
    features.insert(
        "previous_doc_footnote_state".to_owned(),
        bool_as_f64(previous_line.doc_footnote_state),
    );
    features.insert(
        "previous_doc_footnote_continuation".to_owned(),
        bool_as_f64(previous_line.doc_footnote_continuation),
    );
    features.insert(
        "doc_footnote_state_pair".to_owned(),
        bool_as_f64(previous_line.doc_footnote_state && line.doc_footnote_state),
    );
    features.insert(
        "doc_footnote_continuation_pair".to_owned(),
        bool_as_f64(previous_line.doc_footnote_continuation && line.doc_footnote_continuation),
    );
    features.insert(
        "current_doc_note_marker_mid_sequence_page".to_owned(),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
    );
    features.insert(
        "current_doc_note_marker_follows_previous_page".to_owned(),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
    );
    features.insert(
        "current_doc_note_marker_first_on_page".to_owned(),
        bool_as_f64(line.doc_note_marker_first_on_page),
    );
    features.insert(
        "current_doc_note_marker_present".to_owned(),
        bool_as_f64(line.doc_note_marker > 0),
    );
    features.insert(
        "previous_doc_note_marker_present".to_owned(),
        bool_as_f64(previous_line.doc_note_marker > 0),
    );
    features.insert(
        "doc_note_marker_page_delta".to_owned(),
        line.doc_note_marker_page_delta as f64,
    );
    if gap > 0.025 {
        features.insert("vertical_gap_gt_0_025".to_owned(), 1.0);
    }
    if gap > 0.035 {
        features.insert("vertical_gap_gt_0_035".to_owned(), 1.0);
    }
    if let Some(role) = previous_line.role_hint {
        features.insert(
            format!("previous_role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    if let Some(role) = line.role_hint {
        features.insert(
            format!("current_role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    features
}
