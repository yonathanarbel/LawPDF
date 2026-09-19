//! Per-page emission decoding and the fallback Viterbi decoder.

use super::*;

pub(super) fn decode_pages_with_scores(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
) -> Result<(Vec<(DeepLiquidSourceLine, Lm2Action)>, Vec<[f64; 3]>), String> {
    if let Some(model) = runtime.fasttab_model.as_ref() {
        let scores = model
            .emission_scores(lines)
            .map_err(|error| format!("FastTab inference failed: {error}"))?;
        let emissions = validate_emissions(lines, scores.into_iter().map(Ok))?;
        let decoded = lines
            .iter()
            .cloned()
            .zip(emissions.iter().copied().map(argmax_lm2_action))
            .collect();
        return Ok((decoded, emissions));
    }
    if let Some(model) = runtime.native_catboost_model.as_ref() {
        let emissions =
            validate_emissions(lines, lines.iter().map(|line| model.emission_scores(line)))?;
        let decoded = lines
            .iter()
            .cloned()
            .zip(emissions.iter().copied().map(argmax_lm2_action))
            .collect();
        return Ok((decoded, emissions));
    }
    let mut decoded = Vec::with_capacity(lines.len());
    let mut emissions = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    while start < lines.len() {
        let page = lines[start].page_index;
        let mut end = start + 1;
        while end < lines.len() && lines[end].page_index == page {
            end += 1;
        }
        let page_emissions = lines[start..end]
            .iter()
            .map(|line| runtime.emission_scores(line))
            .collect::<Vec<_>>();
        decoded.extend(decode_page_with_emissions(
            runtime,
            &lines[start..end],
            Some(&page_emissions),
        ));
        emissions.extend(page_emissions);
        start = end;
    }
    Ok((decoded, emissions))
}

pub(super) fn validate_emissions(
    lines: &[DeepLiquidSourceLine],
    scores: impl IntoIterator<Item = Result<[f64; 3], String>>,
) -> Result<Vec<[f64; 3]>, String> {
    let mut emissions = Vec::with_capacity(lines.len());
    for (index, result) in scores.into_iter().enumerate() {
        let line = lines
            .get(index)
            .ok_or_else(|| "Model returned too many score rows".to_owned())?;
        let location = || format!("page {}, line {}", line.page_index + 1, line.line_index + 1);
        let values =
            result.map_err(|error| format!("Model inference failed at {}: {error}", location()))?;
        if values.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "Model returned non-finite scores at {}",
                location()
            ));
        }
        emissions.push(values);
    }
    if emissions.len() != lines.len() {
        return Err(format!(
            "Model returned {} score rows for {} source lines",
            emissions.len(),
            lines.len()
        ));
    }
    Ok(emissions)
}

pub(super) fn decode_pages_with_external_emissions(
    runtime: &Lm2Runtime,
    document_path: &Path,
    lines: &[DeepLiquidSourceLine],
    external_emissions: &Lm2ExternalEmissions,
) -> Result<Vec<(DeepLiquidSourceLine, Lm2Action)>, String> {
    let mut decoded = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    while start < lines.len() {
        let page = lines[start].page_index;
        let mut end = start + 1;
        while end < lines.len() && lines[end].page_index == page {
            end += 1;
        }
        let page_scores = external_emissions.page_scores(document_path, &lines[start..end])?;
        decoded.extend(decode_page_with_emissions(
            runtime,
            &lines[start..end],
            Some(&page_scores),
        ));
        start = end;
    }
    Ok(decoded)
}

pub(super) fn decode_page(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
) -> Vec<(DeepLiquidSourceLine, Lm2Action)> {
    decode_page_with_emissions(runtime, lines, None)
}

pub(super) fn decode_page_with_emissions(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
    external_emissions: Option<&[[f64; 3]]>,
) -> Vec<(DeepLiquidSourceLine, Lm2Action)> {
    if lines.is_empty() {
        return Vec::new();
    }
    debug_assert!(
        external_emissions.map_or(true, |emissions| emissions.len() == lines.len()),
        "external LM2 emissions must align one-for-one with decoded lines"
    );
    let emissions = external_emissions
        .map(|scores| scores.to_vec())
        .unwrap_or_else(|| {
            lines
                .iter()
                .map(|line| runtime.emission_scores(line))
                .collect::<Vec<_>>()
        });
    let mut dp = vec![[f64::NEG_INFINITY; 3]; lines.len()];
    let mut back = vec![[0usize; 3]; lines.len()];
    for action in ACTIONS {
        dp[0][action.index()] = emissions[0][action.index()]
            + runtime.start_score_scale * start_cost(lines[0].role_hint, action)
            + decoder_start_correction(runtime, &lines[0], action)
            + decoder_line_prior(runtime, &lines[0], action);
    }
    for index in 1..lines.len() {
        for current in ACTIONS {
            let current_index = current.index();
            let mut best_score = f64::NEG_INFINITY;
            let mut best_prev = 0usize;
            for previous in ACTIONS {
                let previous_index = previous.index();
                let score = dp[index - 1][previous_index]
                    + runtime.transition_score_scale
                        * transition_score(&lines[index - 1], &lines[index], previous, current)
                    + decoder_transition_correction(
                        runtime,
                        &lines[index - 1],
                        &lines[index],
                        previous,
                        current,
                    )
                    + decoder_transition_prior(
                        runtime,
                        &lines[index - 1],
                        &lines[index],
                        previous,
                        current,
                    )
                    + decoder_line_prior(runtime, &lines[index], current)
                    + emissions[index][current_index];
                if score > best_score {
                    best_score = score;
                    best_prev = previous_index;
                }
            }
            dp[index][current_index] = best_score;
            back[index][current_index] = best_prev;
        }
    }
    let mut current = (0..ACTIONS.len())
        .max_by(|left, right| {
            dp[lines.len() - 1][*left]
                .partial_cmp(&dp[lines.len() - 1][*right])
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or(0);
    let mut path = vec![Lm2Action::Keep; lines.len()];
    for index in (0..lines.len()).rev() {
        path[index] = ACTIONS[current];
        if index > 0 {
            current = back[index][current];
        }
    }
    if runtime.anchored_marginalia_flow_guard {
        apply_anchored_marginalia_flow_guard(lines, &mut path);
    }
    if runtime.pp_footnote_region_membership {
        apply_pp_footnote_region_membership(lines, &mut path);
    }
    if runtime.body_preservation_guard {
        apply_body_preservation_guard(lines, &mut path);
    }
    lines
        .iter()
        .cloned()
        .zip(path)
        .map(|(line, action)| {
            let action = final_lm2_action(&line, action);
            (line, action)
        })
        .collect()
}
