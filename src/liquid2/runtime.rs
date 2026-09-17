//! Runtime loading: CatBoost FFI, FastTab, HGB context/arbiter/note-head/link-ranker models, asset discovery, manifest verification.

use super::*;

impl Lm2Runtime {
    pub(super) fn load(runtime_choice: Lm2RuntimeChoice) -> Self {
        let mut load_warnings = Vec::new();
        let pp_priors = load_lm2_pp_priors().ok().flatten();
        let fasttab_requested = runtime_choice.fasttab_requested();
        let fasttab_model = match Lm2FastTabModel::load_requested(fasttab_requested) {
            Ok(Some(model)) => Some(model),
            Ok(None) => {
                if fasttab_requested {
                    load_warnings.push(
                        "Requested FastTab runtime asset was not found; retaining CatBoost default."
                            .to_owned(),
                    );
                }
                None
            }
            Err(error) => {
                load_warnings.push(format!(
                    "Requested FastTab runtime failed to load; retaining CatBoost default: {error}"
                ));
                None
            }
        };
        let native_catboost_model = match load_lm2_native_catboost_model() {
            Ok(Some(model)) => Some(model),
            Ok(None) => {
                load_warnings.push(
                    "Promoted native CatBoost runtime assets were not found; using fallback emissions."
                        .to_owned(),
                );
                None
            }
            Err(error) => {
                load_warnings.push(format!(
                    "Promoted native CatBoost runtime failed to load; using fallback emissions: {error}"
                ));
                None
            }
        };
        let native_line_model_active = fasttab_model.is_some() || native_catboost_model.is_some();
        let context_primary_sha256 = if fasttab_model.is_none() {
            native_catboost_model
                .as_ref()
                .map(|model| model.model_sha256.as_str())
        } else {
            None
        };
        let context_twopass_model = if native_line_model_active && lm2_context_twopass_enabled() {
            match load_lm2_context_twopass_model(context_primary_sha256) {
                Ok(Some(model)) => Some(model),
                Ok(None) => {
                    load_warnings.push(
                        "Promoted context two-pass model was not found; using native emissions only."
                            .to_owned(),
                    );
                    None
                }
                Err(error) => {
                    load_warnings.push(format!(
                        "Promoted context two-pass model failed to load; using native emissions only: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let context_arbiter_model = if native_catboost_model.is_some()
            && fasttab_model.is_none()
            && lm2_context_twopass_enabled()
        {
            match load_lm2_context_arbiter_model(
                context_primary_sha256,
                context_twopass_model
                    .as_ref()
                    .map(|model| model.asset_sha256.as_str()),
            ) {
                Ok(model) => model,
                Err(error) => {
                    load_warnings.push(format!(
                        "Optional context arbiter failed to load; retaining the promoted context model only: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let note_head_model = if native_line_model_active {
            match load_lm2_note_head_model(context_primary_sha256) {
                Ok(model) => model,
                Err(error) => {
                    load_warnings.push(format!(
                        "Optional learned note-head scorer failed to load: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let link_ranker_model = if native_line_model_active {
            match load_lm2_link_ranker_model(note_head_model.as_ref()) {
                Ok(model) => model,
                Err(error) => {
                    load_warnings.push(format!(
                        "Optional learned footnote link ranker failed to load: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let numeric_catboost_model = if native_line_model_active {
            None
        } else {
            load_lm2_numeric_catboost_model().ok().flatten()
        };
        let static_front_overlay = if native_line_model_active {
            None
        } else {
            load_lm2_static_front_overlay().ok().flatten()
        };
        let runtime_label = fasttab_model
            .as_ref()
            .map(Lm2FastTabModel::label)
            .or_else(|| {
                native_catboost_model.as_ref().map(|model| {
                    format!(
                        "lm2-native-catboost-text-runtime:f{}c{}t{}d{}:sha{}",
                        model.float_feature_count,
                        model.cat_feature_count,
                        model.text_feature_count,
                        model.dimensions_count,
                        &model.model_sha256[..16]
                    )
                })
            })
            .or_else(|| {
                numeric_catboost_model.as_ref().map(|_| {
                    let source =
                        std::env::var("LAWPDF_LM2_NUMERIC_CATBOOST_MODEL").unwrap_or_default();
                    format!("lm2-numeric-catboost-runtime:{:016x}", fnv1a64(&source))
                })
            });
        match load_lm2_model() {
            Ok(model) if model_is_usable(&model) => Self {
                model_label: runtime_label.unwrap_or_else(|| model.model_id.clone()),
                load_warnings,
                model: Some(model),
                fasttab_model,
                native_catboost_model,
                context_twopass_model,
                context_arbiter_model,
                note_head_model,
                link_ranker_model,
                numeric_catboost_model,
                static_front_overlay,
                pp_priors,
                pp_footnote_region_membership: !native_line_model_active
                    && lm2_pp_footnote_region_membership_enabled(),
                marker_decoder_prior: !native_line_model_active
                    && lm2_marker_decoder_prior_enabled(),
                small_font_decoder_prior: !native_line_model_active
                    && lm2_small_font_decoder_prior_enabled(),
                small_font_sequence_prior: !native_line_model_active
                    && lm2_small_font_sequence_prior_enabled(),
                anchored_marginalia_flow_guard: !native_line_model_active
                    && lm2_anchored_marginalia_flow_guard_enabled(),
                body_preservation_guard: !native_line_model_active
                    && lm2_body_preservation_guard_enabled(),
                action_neutral_blocksplit: !native_line_model_active
                    && lm2_action_neutral_blocksplit_enabled(),
                toc_overlay: !native_line_model_active && lm2_toc_overlay_enabled(),
                front_matter_guard: !native_line_model_active && lm2_front_matter_guard_enabled(),
                marginalia_preservation_guard: !native_line_model_active
                    && lm2_marginalia_preservation_guard_enabled(),
                start_score_scale: if native_line_model_active {
                    1.0
                } else {
                    lm2_start_score_scale()
                },
                transition_score_scale: if native_line_model_active {
                    1.0
                } else {
                    lm2_transition_score_scale()
                },
            },
            _ => Self {
                model: None,
                fasttab_model,
                model_label: runtime_label.unwrap_or_else(|| "lm2-heuristic-fallback".to_owned()),
                load_warnings,
                native_catboost_model,
                context_twopass_model,
                context_arbiter_model,
                note_head_model,
                link_ranker_model,
                numeric_catboost_model,
                static_front_overlay,
                pp_priors,
                pp_footnote_region_membership: !native_line_model_active
                    && lm2_pp_footnote_region_membership_enabled(),
                marker_decoder_prior: !native_line_model_active
                    && lm2_marker_decoder_prior_enabled(),
                small_font_decoder_prior: !native_line_model_active
                    && lm2_small_font_decoder_prior_enabled(),
                small_font_sequence_prior: !native_line_model_active
                    && lm2_small_font_sequence_prior_enabled(),
                anchored_marginalia_flow_guard: !native_line_model_active
                    && lm2_anchored_marginalia_flow_guard_enabled(),
                body_preservation_guard: !native_line_model_active
                    && lm2_body_preservation_guard_enabled(),
                action_neutral_blocksplit: !native_line_model_active
                    && lm2_action_neutral_blocksplit_enabled(),
                toc_overlay: !native_line_model_active && lm2_toc_overlay_enabled(),
                front_matter_guard: !native_line_model_active && lm2_front_matter_guard_enabled(),
                marginalia_preservation_guard: !native_line_model_active
                    && lm2_marginalia_preservation_guard_enabled(),
                start_score_scale: if native_line_model_active {
                    1.0
                } else {
                    lm2_start_score_scale()
                },
                transition_score_scale: if native_line_model_active {
                    1.0
                } else {
                    lm2_transition_score_scale()
                },
            },
        }
    }

    pub(super) fn emission_scores(&self, line: &DeepLiquidSourceLine) -> [f64; 3] {
        if let Some(model) = self.fasttab_model.as_ref()
            && let Ok(scores) = model.emission_scores(std::slice::from_ref(line))
            && let Some(scores) = scores.first()
        {
            return *scores;
        }
        if let Some(model) = self.native_catboost_model.as_ref() {
            return model.emission_scores(line).unwrap_or([0.0, 0.0, 0.0]);
        }
        let mut scores = if let Some(model) = self.numeric_catboost_model.as_ref() {
            model.emission_scores(line)
        } else if let Some(model) = self.model.as_ref() {
            let features = lm2_features(
                line,
                model.feature_dim,
                model.doc_font_zscores_enabled(),
                model.repetition_fingerprints_enabled(),
                model.marker_continuity_enabled(),
            );
            let mut scores = [0.0, 0.0, 0.0];
            for (index, score) in scores.iter_mut().enumerate() {
                *score = *model.bias.get(index).unwrap_or(&0.0);
            }
            for (feature_index, value) in features {
                for (class_index, score) in scores.iter_mut().enumerate() {
                    if let Some(weight) = model
                        .weights
                        .get(class_index)
                        .and_then(|weights| weights.get(feature_index))
                    {
                        *score += weight * value;
                    }
                }
            }
            scores
        } else {
            [0.0, 0.0, 0.0]
        };
        apply_layout_priors(line, &mut scores);
        apply_pp_priors(self, line, &mut scores);
        scores
    }

    pub(super) fn native_line_model_active(&self) -> bool {
        self.fasttab_model.is_some() || self.native_catboost_model.is_some()
    }

    pub(super) fn decoder_weights(&self) -> Option<&HashMap<String, f64>> {
        self.model
            .as_ref()
            .and_then(|model| model.decoder_constants.as_ref())
            .map(|constants| &constants.weights)
            .filter(|weights| !weights.is_empty())
    }

    pub(super) fn pp_prior_for_line(&self, line: &DeepLiquidSourceLine) -> Option<Lm2PpPrior> {
        line.pp_prior_score.map(|score| Lm2PpPrior {
            role: line.pp_prior_role.clone().unwrap_or_default(),
            label: line.pp_prior_label.clone().unwrap_or_default(),
            score,
        })
    }

    pub(super) fn pp_prior_source(&self) -> Option<String> {
        self.pp_priors
            .as_ref()
            .map(|index| index.source.display().to_string())
    }
}

pub(super) fn argmax_lm2_action(scores: [f64; 3]) -> Lm2Action {
    ACTIONS
        .into_iter()
        .max_by(|left, right| {
            scores[left.index()]
                .partial_cmp(&scores[right.index()])
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or(Lm2Action::Keep)
}

impl Lm2ContextTwopassModel {
    pub(super) fn label(&self) -> String {
        if matches!(
            self.schema_version.as_str(),
            LM2_CONTEXT_ARBITER_SCHEMA_V2 | LM2_CONTEXT_RESIDUAL_SCHEMA_V3
        ) {
            let primary = self
                .primary_model_sha256
                .as_deref()
                .unwrap_or("unknown")
                .chars()
                .take(12)
                .collect::<String>();
            let asset = self.asset_sha256.chars().take(12).collect::<String>();
            let runtime_version = if self.schema_version == LM2_CONTEXT_RESIDUAL_SCHEMA_V3 {
                LM2_CONTEXT_RESIDUAL_RUNTIME_VERSION
            } else {
                LM2_CONTEXT_ARBITER_RUNTIME_VERSION
            };
            return format!(
                "{}:{}:a{}f{}m{}:primary{}:asset{}",
                runtime_version,
                self.schema_version,
                self.actions.len(),
                self.feature_count,
                self.models.len(),
                primary,
                asset
            );
        }
        format!(
            "{LM2_CONTEXT_TWOPASS_VERSION}:{}:a{}f{}m{}:asset{}",
            self.schema_version,
            self.actions.len(),
            self.feature_count,
            self.models.len(),
            self.asset_sha256,
        )
    }

    pub(super) fn model_for_document(
        &self,
        document_path: &Path,
    ) -> Option<&Lm2ContextTwopassHgbModel> {
        let normalized = lm2_context_norm_path(&document_path.display().to_string());
        if let Some(fold) = self.doc_to_fold.get(&normalized) {
            let name = format!("fold{fold}");
            if let Some(model) = self.models.iter().find(|model| model.name == name) {
                return Some(model);
            }
        }
        self.models
            .iter()
            .find(|model| model.name == self.unseen_doc_model)
            .or_else(|| self.models.first())
    }
}

impl Lm2ContextTwopassHgbModel {
    pub(super) fn raw_prediction(&self, features: &[f64]) -> [f64; 3] {
        let mut raw = [0.0; 3];
        for (index, score) in raw.iter_mut().enumerate() {
            *score = *self.baseline_prediction.get(index).unwrap_or(&0.0);
        }
        for iteration in &self.trees {
            for (class_index, tree) in iteration.iter().take(3).enumerate() {
                raw[class_index] += lm2_context_tree_predict(tree, features);
            }
        }
        raw
    }

    pub(super) fn predict(&self, features: &[f64]) -> Lm2Action {
        let raw = self.raw_prediction(features);
        let class_index = raw
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.partial_cmp(right).unwrap_or(Ordering::Equal))
            .map(|(index, _)| index)
            .unwrap_or(1);
        match class_index {
            0 => Lm2Action::HideNoise,
            1 => Lm2Action::Keep,
            2 => Lm2Action::Marginalia,
            _ => Lm2Action::Keep,
        }
    }

    pub(super) fn predict_probabilities(&self, features: &[f64]) -> [f64; 3] {
        lm2_context_softmax(self.raw_prediction(features))
    }
}

impl Lm2NoteHeadModel {
    pub(super) fn label(&self) -> String {
        let asset = self.asset_sha256.chars().take(12).collect::<String>();
        let primary = self
            .primary_model_sha256
            .as_deref()
            .map(|sha| sha.chars().take(12).collect::<String>())
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "{LM2_NOTE_HEAD_RUNTIME_VERSION}:{}:f{}t{}:primary{primary}:asset{asset}",
            self.schema_version,
            self.feature_count,
            self.trees.len(),
        )
    }

    pub(super) fn probability(&self, features: &[f64]) -> f64 {
        let raw = self
            .trees
            .iter()
            .fold(self.baseline_prediction, |score, tree| {
                score + lm2_context_tree_predict(tree, features)
            });
        if raw >= 0.0 {
            1.0 / (1.0 + (-raw).exp())
        } else {
            let exp = raw.exp();
            exp / (1.0 + exp)
        }
    }
}

impl Lm2LinkRankerModel {
    pub(super) fn label(&self) -> String {
        let asset = self.asset_sha256.chars().take(12).collect::<String>();
        let auth = self.auth_model_sha256.chars().take(12).collect::<String>();
        format!(
            "{LM2_LINK_RANKER_RUNTIME_VERSION}:{LM2_LINK_RANKER_SCHEMA_V1}:f{}t{}:auth{auth}:asset{asset}",
            LM2_LINK_RANKER_FEATURE_COUNT_V1,
            self.trees.len(),
        )
    }

    pub(super) fn probability(&self, features: &[f64]) -> f64 {
        let raw = self
            .trees
            .iter()
            .fold(self.baseline_prediction, |score, tree| {
                score + lm2_context_tree_predict(tree, features)
            });
        if raw >= 0.0 {
            1.0 / (1.0 + (-raw).exp())
        } else {
            let exp = raw.exp();
            exp / (1.0 + exp)
        }
    }
}

pub(super) fn lm2_context_softmax(raw: [f64; 3]) -> [f64; 3] {
    let max = raw.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let exp = raw.map(|value| (value - max).exp());
    let total = exp.iter().sum::<f64>();
    if total.is_finite() && total > 0.0 {
        exp.map(|value| value / total)
    } else {
        [0.0, 1.0, 0.0]
    }
}

pub(super) fn lm2_context_tree_predict(nodes: &[[f64; 7]], features: &[f64]) -> f64 {
    let mut index = 0usize;
    for _ in 0..nodes.len() {
        let Some(node) = nodes.get(index) else {
            return 0.0;
        };
        if node[6] != 0.0 {
            return node[0];
        }
        let feature_index = node[1] as usize;
        let threshold = node[2];
        let value = *features.get(feature_index).unwrap_or(&0.0);
        let go_left = if value.is_nan() {
            node[3] != 0.0
        } else {
            value <= threshold
        };
        index = if go_left {
            node[4] as usize
        } else {
            node[5] as usize
        };
    }
    0.0
}

pub(super) fn apply_context_twopass_model(
    model: &Lm2ContextTwopassModel,
    document_path: &Path,
    primary_emissions: Option<&[[f64; 3]]>,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.is_empty() {
        return;
    }
    let Some(document_model) = model.model_for_document(document_path) else {
        return;
    };
    if matches!(
        model.schema_version.as_str(),
        LM2_CONTEXT_ARBITER_SCHEMA_V2 | LM2_CONTEXT_RESIDUAL_SCHEMA_V3
    ) {
        let Some(primary_emissions) = primary_emissions.filter(|rows| rows.len() == decoded.len())
        else {
            return;
        };
        let baseline_actions = decoded
            .iter()
            .map(|(_, action)| *action)
            .collect::<Vec<_>>();
        let primary_probabilities = primary_emissions
            .iter()
            .copied()
            .map(lm2_context_primary_probabilities)
            .collect::<Vec<_>>();
        let Some(calibration) = model.calibration.as_ref() else {
            return;
        };
        for index in 0..decoded.len() {
            let features = if model.schema_version == LM2_CONTEXT_RESIDUAL_SCHEMA_V3 {
                lm2_context_runtime_residual_feature_vector(
                    decoded,
                    &primary_probabilities,
                    baseline_actions[index],
                    index,
                )
            } else {
                lm2_context_arbiter_feature_vector(decoded, &primary_probabilities, index)
            };
            if features.len() != model.feature_count {
                continue;
            }
            let arbiter = document_model.predict_probabilities(&features);
            let prediction = if model.schema_version == LM2_CONTEXT_RESIDUAL_SCHEMA_V3 {
                lm2_context_runtime_residual_policy(
                    baseline_actions[index],
                    arbiter,
                    calibration,
                    &decoded[index].0.text,
                )
            } else {
                lm2_context_arbiter_policy(baseline_actions[index], arbiter, calibration)
            };
            decoded[index].1 = prediction;
        }
        return;
    }
    let baseline_actions = decoded
        .iter()
        .map(|(_, action)| *action)
        .collect::<Vec<_>>();
    let block_meta = lm2_context_block_meta(decoded);
    for index in 0..decoded.len() {
        let features = lm2_context_feature_vector(decoded, &baseline_actions, &block_meta, index);
        if features.len() == model.feature_count {
            decoded[index].1 = document_model.predict(&features);
        }
    }
}

pub(super) fn lm2_context_primary_probabilities(internal_scores: [f64; 3]) -> [f64; 3] {
    // Native inference is [keep, marginalia, hide_noise], whereas the arbiter's
    // training/export contract is [hide_noise, keep, marginalia].
    lm2_context_softmax([
        internal_scores[Lm2Action::HideNoise.index()],
        internal_scores[Lm2Action::Keep.index()],
        internal_scores[Lm2Action::Marginalia.index()],
    ])
}

pub(super) fn lm2_context_finite_feature(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else if value == f64::INFINITY {
        1_000_000.0
    } else if value == f64::NEG_INFINITY {
        -1_000_000.0
    } else {
        value
    }
}

pub(super) fn lm2_context_arbiter_feature_vector(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    primary_probabilities: &[[f64; 3]],
    index: usize,
) -> Vec<f64> {
    let mut features = Vec::with_capacity(LM2_CONTEXT_ARBITER_FEATURE_COUNT_V2);
    let feature_map = lm2_numeric_catboost_features(&decoded[index].0);
    features.extend(
        LM2_NATIVE_CATBOOST_FLOAT_FEATURES
            .iter()
            .map(|name| lm2_context_finite_feature(feature_map.get(*name).copied().unwrap_or(0.0))),
    );
    features.extend(primary_probabilities[index]);

    let mut neighbor_meta = Vec::with_capacity(LM2_CONTEXT_ARBITER_NEIGHBOR_OFFSETS.len());
    for offset in LM2_CONTEXT_ARBITER_NEIGHBOR_OFFSETS {
        let neighbor = index.checked_add_signed(offset).filter(|neighbor| {
            decoded
                .get(*neighbor)
                .is_some_and(|(line, _)| line.page_index == decoded[index].0.page_index)
        });
        if let Some(neighbor) = neighbor {
            features.extend(primary_probabilities[neighbor]);
            let gap = decoded[index]
                .0
                .line_index
                .abs_diff(decoded[neighbor].0.line_index)
                .min(8) as f64
                / 8.0;
            neighbor_meta.push((1.0, gap));
        } else {
            features.extend([0.0; 3]);
            neighbor_meta.push((0.0, 0.0));
        }
    }
    features.extend(neighbor_meta.iter().map(|(same_page, _)| *same_page));
    features.extend(neighbor_meta.iter().map(|(_, gap)| *gap));
    features
}

pub(super) fn lm2_context_runtime_residual_feature_vector(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    primary_probabilities: &[[f64; 3]],
    baseline_action: Lm2Action,
    index: usize,
) -> Vec<f64> {
    let mut features = lm2_context_arbiter_feature_vector(decoded, primary_probabilities, index);
    features.reserve(3);
    lm2_context_onehot(&mut features, Some(baseline_action));
    features
}

pub(super) fn lm2_context_has_substantive_text(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

pub(super) fn lm2_note_head_feature_vector(
    line: &DeepLiquidSourceLine,
    primary_probabilities: Option<[f64; 3]>,
) -> Option<(u16, Vec<f64>)> {
    let (marker, normalized_text) = lm2_note_head_normalized_text(&line.text)?;
    let trimmed = normalized_text.as_str();
    let digit_count = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .count();
    let suffix = trimmed.chars().nth(digit_count)?;
    let remainder = trimmed
        .chars()
        .skip(digit_count + 1)
        .skip_while(|ch| ch.is_whitespace())
        .collect::<String>();
    let first = remainder.chars().next();
    let text_chars = normalized_text.chars().count().max(1);
    let text_digits = normalized_text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .count();
    let token_count = normalized_text.split_whitespace().count();
    let mut features =
        Vec::with_capacity(LM2_NOTE_HEAD_FEATURE_COUNT_V1 + primary_probabilities.map_or(0, |_| 3));
    let feature_map = lm2_numeric_catboost_features(line);
    features.extend(
        LM2_NATIVE_CATBOOST_FLOAT_FEATURES
            .iter()
            .map(|name| lm2_context_finite_feature(feature_map.get(*name).copied().unwrap_or(0.0))),
    );
    if let Some(probabilities) = primary_probabilities {
        features.extend(probabilities);
    }
    features.extend([
        marker as f64 / 500.0,
        (marker as f64).ln_1p() / 501.0f64.ln(),
        digit_count as f64 / 3.0,
        text_chars.min(500) as f64 / 500.0,
        token_count.min(100) as f64 / 100.0,
        text_digits as f64 / text_chars as f64,
        bool_as_f64(suffix.is_whitespace()),
        bool_as_f64(suffix == '.'),
        bool_as_f64(suffix == ')'),
        bool_as_f64(suffix == ']'),
        bool_as_f64(first.is_some_and(char::is_uppercase)),
        bool_as_f64(first.is_some_and(char::is_lowercase)),
        bool_as_f64(first.is_some_and(|ch| ch.is_ascii_digit())),
        bool_as_f64(marker <= 5),
        bool_as_f64(marker >= 100),
    ]);
    Some((marker, features))
}

pub(super) fn lm2_note_head_normalized_text(text: &str) -> Option<(u16, String)> {
    let trimmed = text.trim_start();
    if let Some(marker) = leading_numeric_token_marker(trimmed) {
        return Some((marker, trimmed.to_owned()));
    }
    let mut chars = trimmed.char_indices();
    if chars.next().map(|(_, ch)| ch) != Some(CALLOUT_START) {
        return None;
    }
    let mut digits = String::new();
    let mut end = None;
    for (index, ch) in chars {
        if ch == CALLOUT_END {
            end = Some(index + ch.len_utf8());
            break;
        }
        if !ch.is_ascii_digit() || digits.len() >= 3 {
            return None;
        }
        digits.push(ch);
    }
    let marker = digits.parse::<u16>().ok()?;
    if !(1..=LM2_MAX_NOTE_MARKER).contains(&marker) {
        return None;
    }
    let remainder = trimmed.get(end?..)?.trim_start();
    Some((marker, format!("{marker} {remainder}")))
}

pub(super) fn lm2_note_head_sequence_features(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    primary_probabilities: &[[f64; 3]],
) -> Vec<Option<[f64; 16]>> {
    let mut candidates = decoded
        .iter()
        .enumerate()
        .filter_map(|(index, (line, _))| {
            lm2_note_head_normalized_text(&line.text)
                .map(|(marker, _)| (index, marker, line.page_index, line.line_index))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(index, _, page, line)| (*page, *line, *index));
    let mut output = vec![None; decoded.len()];
    for (position, (index, marker, page, line)) in candidates.iter().copied().enumerate() {
        let mut features = [0.0; 16];
        for (slot, offset) in LM2_NOTE_HEAD_NEIGHBOR_OFFSETS_V3.into_iter().enumerate() {
            let Some(neighbor_position) = position.checked_add_signed(offset) else {
                continue;
            };
            let Some((neighbor_index, neighbor_marker, neighbor_page, neighbor_line)) =
                candidates.get(neighbor_position).copied()
            else {
                continue;
            };
            let cursor = slot * 8;
            let same_page = neighbor_page == page;
            features[cursor] = 1.0;
            features[cursor + 1] =
                (neighbor_marker as i32 - marker as i32).clamp(-20, 20) as f64 / 20.0;
            features[cursor + 2] = (neighbor_page as i64 - page as i64).clamp(-4, 4) as f64 / 4.0;
            features[cursor + 3] = if same_page {
                neighbor_line.abs_diff(line).min(50) as f64 / 50.0
            } else {
                1.0
            };
            features[cursor + 4] = bool_as_f64(same_page);
            if let Some(probabilities) = primary_probabilities.get(neighbor_index) {
                features[cursor + 5..cursor + 8].copy_from_slice(probabilities);
            }
        }
        output[index] = Some(features);
    }
    output
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Lm2LinkNumericCandidate {
    pub(super) decoded_index: usize,
    pub(super) marker: u16,
}

pub(super) fn lm2_link_numeric_candidates(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> Vec<Lm2LinkNumericCandidate> {
    let mut candidates = decoded
        .iter()
        .enumerate()
        .filter_map(|(decoded_index, (line, _))| {
            lm2_note_head_normalized_text(&line.text).map(|(marker, _)| Lm2LinkNumericCandidate {
                decoded_index,
                marker,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| {
        let line = &decoded[candidate.decoded_index].0;
        (line.page_index, line.line_index, candidate.decoded_index)
    });
    candidates
}

pub(super) fn lm2_note_head_auth_probabilities(
    model: &Lm2NoteHeadModel,
    document_path: &Path,
    primary_emissions: &[[f64; 3]],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> Vec<Option<(u16, f64)>> {
    if model.schema_version != LM2_NOTE_HEAD_SCHEMA_V3 || primary_emissions.len() != decoded.len() {
        return vec![None; decoded.len()];
    }
    let primary_probabilities = primary_emissions
        .iter()
        .copied()
        .map(lm2_context_primary_probabilities)
        .collect::<Vec<_>>();
    let sequence_features = lm2_note_head_sequence_features(decoded, &primary_probabilities);
    let trace_path = std::env::var_os("LAWPDF_LM2_TRACE_NOTE_HEAD_FILE").map(PathBuf::from);
    let mut trace_rows = Vec::new();
    let mut output = Vec::with_capacity(decoded.len());
    for (index, (line, action)) in decoded.iter().enumerate() {
        let Some((marker, mut features)) =
            lm2_note_head_feature_vector(line, Some(primary_probabilities[index]))
        else {
            output.push(None);
            continue;
        };
        if let Some(sequence) = sequence_features.get(index).copied().flatten() {
            features.extend(sequence);
        }
        if features.len() != model.feature_count {
            output.push(None);
            continue;
        }
        let probability = model.probability(&features);
        if trace_path.is_some() {
            trace_rows.push(serde_json::json!({
                "document": document_path.display().to_string(),
                "id": line.id.clone(),
                "page_index": line.page_index,
                "line_index": line.line_index,
                "marker": marker,
                "probability": probability,
                "threshold": model.threshold,
                "primary_probabilities": primary_probabilities[index],
                "features": features,
                "baseline_action": action.as_str(),
                "text": line.text.clone(),
            }));
        }
        output.push(Some((marker, probability)));
    }
    if let Some(path) = trace_path
        && !trace_rows.is_empty()
        && let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write as _;
        let mut writer = std::io::BufWriter::new(file);
        for row in trace_rows {
            if let Ok(encoded) = serde_json::to_string(&row) {
                let _ = writeln!(writer, "{encoded}");
            }
        }
        let _ = writer.flush();
    }
    output
}

pub(super) fn lm2_link_text_contains_marker(text: &str, marker: u16) -> bool {
    if sentineled_note_markers(text).contains(&marker) {
        return true;
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() && index - start < 3 {
            index += 1;
        }
        let bounded_left = start == 0 || !chars[start - 1].is_ascii_digit();
        let bounded_right = index == chars.len() || !chars[index].is_ascii_digit();
        let value = chars[start..index]
            .iter()
            .collect::<String>()
            .parse::<u16>()
            .ok();
        if bounded_left && bounded_right && value == Some(marker) {
            return true;
        }
    }
    false
}

pub(super) fn lm2_link_neighbor_features(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    candidate: Lm2LinkNumericCandidate,
    neighbor: Option<Lm2LinkNumericCandidate>,
) -> [f64; 6] {
    let Some(neighbor) = neighbor else {
        return [0.0; 6];
    };
    let line = &decoded[candidate.decoded_index].0;
    let other = &decoded[neighbor.decoded_index].0;
    let same_page = line.page_index == other.page_index;
    [
        1.0,
        (neighbor.marker as i32 - candidate.marker as i32).clamp(-20, 20) as f64 / 20.0,
        (other.page_index as i64 - line.page_index as i64).clamp(-4, 4) as f64 / 4.0,
        if same_page {
            other.line_index.abs_diff(line.line_index).min(100) as f64 / 100.0
        } else {
            1.0
        },
        bool_as_f64(same_page),
        bool_as_f64(decoded[neighbor.decoded_index].1 == Lm2Action::Marginalia),
    ]
}

pub(super) fn lm2_link_note_neighbor_features(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    candidate: Lm2LinkNumericCandidate,
    neighbor: Option<Lm2LinkNumericCandidate>,
) -> [f64; 4] {
    let Some(neighbor) = neighbor else {
        return [0.0; 4];
    };
    let line = &decoded[candidate.decoded_index].0;
    let other = &decoded[neighbor.decoded_index].0;
    let same_page = line.page_index == other.page_index;
    [
        1.0,
        (neighbor.marker as i32 - candidate.marker as i32).clamp(-20, 20) as f64 / 20.0,
        (other.page_index as i64 - line.page_index as i64).clamp(-8, 8) as f64 / 8.0,
        if same_page {
            other.line_index.abs_diff(line.line_index).min(100) as f64 / 100.0
        } else {
            1.0
        },
    ]
}

pub(super) fn lm2_link_ranker_feature_vector(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    candidates: &[Lm2LinkNumericCandidate],
    candidate_position: usize,
    reference_index: usize,
    local_same_marker_count: usize,
    document_same_marker_count: usize,
    auth_probability: f64,
) -> Vec<f64> {
    let candidate = candidates[candidate_position];
    let line = &decoded[candidate.decoded_index].0;
    let reference = &decoded[reference_index].0;
    let page_count = decoded
        .iter()
        .map(|(line, _)| line.page_index)
        .max()
        .map_or(1, |page| page + 1);
    let page_max_line = decoded
        .iter()
        .filter(|(other, _)| other.page_index == line.page_index)
        .map(|(other, _)| other.line_index)
        .max()
        .unwrap_or(line.line_index)
        .max(1);
    let width = (line.page_width as f64).max(1.0);
    let height = (line.page_height as f64).max(1.0);
    let page_delta = line.page_index as i64 - reference.page_index as i64;
    let (_, normalized_text) =
        lm2_note_head_normalized_text(&line.text).unwrap_or((candidate.marker, line.text.clone()));
    let digit_count = normalized_text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .count();
    let suffix = normalized_text.chars().nth(digit_count).unwrap_or(' ');
    let remainder = normalized_text
        .chars()
        .skip(digit_count + 1)
        .skip_while(|ch| ch.is_whitespace())
        .collect::<String>();
    let text_chars = normalized_text.chars().count().max(1);
    let digit_chars = normalized_text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .count();
    let alpha_chars = normalized_text
        .chars()
        .filter(|ch| ch.is_alphabetic())
        .collect::<Vec<_>>();
    let uppercase_chars = alpha_chars.iter().filter(|ch| ch.is_uppercase()).count();
    let punctuation_chars = normalized_text
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    let whitespace_chars = normalized_text
        .chars()
        .filter(|ch| ch.is_whitespace())
        .count();
    let reference_starts = lm2_note_head_normalized_text(&reference.text)
        .is_some_and(|(marker, _)| marker == candidate.marker);
    let reference_has_marker = lm2_link_text_contains_marker(&reference.text, candidate.marker);
    let reference_words = reference.text.split_whitespace().count();
    let same_reference_page = line.page_index == reference.page_index;
    let segment_count = line.segment_block_line_count.max(1);
    let previous = candidate_position
        .checked_sub(1)
        .and_then(|position| candidates.get(position))
        .copied();
    let next = candidates.get(candidate_position + 1).copied();
    let coordinate = (line.page_index, line.line_index, candidate.decoded_index);
    let current_notes = candidates
        .iter()
        .copied()
        .filter(|other| decoded[other.decoded_index].1 == Lm2Action::Marginalia)
        .collect::<Vec<_>>();
    let previous_note = current_notes.iter().copied().rev().find(|other| {
        let other_line = &decoded[other.decoded_index].0;
        (
            other_line.page_index,
            other_line.line_index,
            other.decoded_index,
        ) < coordinate
    });
    let next_note = current_notes.iter().copied().find(|other| {
        let other_line = &decoded[other.decoded_index].0;
        (
            other_line.page_index,
            other_line.line_index,
            other.decoded_index,
        ) > coordinate
    });
    let previous_consecutive =
        previous_note.is_some_and(|other| other.marker + 1 == candidate.marker);
    let next_consecutive = next_note.is_some_and(|other| candidate.marker + 1 == other.marker);
    let first_remainder = remainder.chars().next();
    let lower_remainder = remainder.to_lowercase();
    let terminal_punctuation = remainder
        .trim_end()
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ';' | ':'));

    let mut features = vec![
        candidate.marker as f64 / 500.0,
        (candidate.marker as f64).ln_1p() / 501.0f64.ln(),
        candidate.marker.to_string().len() as f64 / 3.0,
        bool_as_f64(candidate.marker <= 5),
        bool_as_f64(candidate.marker >= 100),
        page_delta.clamp(-4, 4) as f64 / 4.0,
        bool_as_f64(page_delta == 0),
        bool_as_f64(page_delta == 1),
        reference.page_index as f64 / page_count.saturating_sub(1).max(1) as f64,
        line.page_index as f64 / page_count.saturating_sub(1).max(1) as f64,
        1.0,
        bool_as_f64(reference_starts),
        bool_as_f64(reference_has_marker && !reference_starts),
        reference_words.min(100) as f64 / 100.0,
        reference.font_ratio_page as f64,
        bool_as_f64(line.id == reference.id),
        bool_as_f64(
            (line.page_index, line.line_index) > (reference.page_index, reference.line_index),
        ),
        if same_reference_page {
            (line.line_index as i64 - reference.line_index as i64).clamp(-100, 100) as f64 / 100.0
        } else {
            0.0
        },
        line.line_index as f64 / page_max_line as f64,
        line.top as f64 / height,
        line.bottom as f64 / height,
        ((line.top + line.bottom) as f64 * 0.5) / height,
        (line.right - line.left).max(0.0) as f64 / width,
        line.left as f64 / width,
        (line.page_width - line.right).max(0.0) as f64 / width,
        line.font_ratio_page as f64,
        line.font_ratio_page_ref as f64,
        line.font_ratio_doc as f64,
        line.font_height as f64 / height,
        bool_as_f64(line.bold),
        bool_as_f64(line.italic),
        bool_as_f64(line.centered),
        bool_as_f64(line.margin_centered),
        bool_as_f64(line.below_footnote_divider),
        bool_as_f64(line.page_has_footnote_divider),
        bool_as_f64(line.in_footnote_zone),
        bool_as_f64(line.doc_footnote_state),
        bool_as_f64(line.doc_footnote_continuation),
        bool_as_f64(line.segment_block_footnote_like),
        bool_as_f64(line.segment_block_furniture_like),
        bool_as_f64(line.segment_block_table_like),
        bool_as_f64(line.segment_block_toc_like),
        bool_as_f64(line.segment_block_first),
        bool_as_f64(line.segment_block_last),
        line.segment_block_line_index as f64 / segment_count.saturating_sub(1).max(1) as f64,
        segment_count.min(50) as f64 / 50.0,
        text_chars.min(500) as f64 / 500.0,
        normalized_text.split_whitespace().count().min(100) as f64 / 100.0,
        digit_chars as f64 / text_chars as f64,
        alpha_chars.len() as f64 / text_chars as f64,
        uppercase_chars as f64 / alpha_chars.len().max(1) as f64,
        punctuation_chars as f64 / text_chars as f64,
        whitespace_chars as f64 / text_chars as f64,
        bool_as_f64(suffix.is_whitespace()),
        bool_as_f64(suffix == '.'),
        bool_as_f64(suffix == ')'),
        bool_as_f64(suffix == ']'),
        bool_as_f64(first_remainder.is_some_and(char::is_uppercase)),
        bool_as_f64(first_remainder.is_some_and(char::is_lowercase)),
        bool_as_f64(first_remainder.is_some_and(|ch| ch.is_ascii_digit())),
        bool_as_f64(
            lower_remainder.contains("http://")
                || lower_remainder.contains("https://")
                || lower_remainder.contains("www."),
        ),
        bool_as_f64(terminal_punctuation),
        bool_as_f64(
            uppercase_chars as f64 / alpha_chars.len().max(1) as f64 >= 0.72
                && remainder.chars().count() >= 8,
        ),
        local_same_marker_count.min(8) as f64 / 8.0,
        document_same_marker_count.min(20) as f64 / 20.0,
    ];
    features.extend(lm2_link_neighbor_features(decoded, candidate, previous));
    features.extend(lm2_link_neighbor_features(decoded, candidate, next));
    features.extend(lm2_link_note_neighbor_features(
        decoded,
        candidate,
        previous_note,
    ));
    features.extend(lm2_link_note_neighbor_features(
        decoded, candidate, next_note,
    ));
    features.extend([
        bool_as_f64(previous_consecutive),
        bool_as_f64(next_consecutive),
        bool_as_f64(previous_consecutive && next_consecutive),
        auth_probability,
    ]);
    features
}

pub(super) fn apply_footnote_link_ranker(
    link_model: &Lm2LinkRankerModel,
    auth_model: &Lm2NoteHeadModel,
    document_path: &Path,
    primary_emissions: Option<&[[f64; 3]]>,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let Some(primary_emissions) = primary_emissions.filter(|rows| rows.len() == decoded.len())
    else {
        return 0;
    };
    let candidates = lm2_link_numeric_candidates(decoded);
    if candidates.is_empty() {
        return 0;
    }
    let auth_probabilities =
        lm2_note_head_auth_probabilities(auth_model, document_path, primary_emissions, decoded);
    let mut candidate_counts_by_marker = HashMap::<u16, usize>::new();
    let mut candidate_positions_by_marker = HashMap::<u16, Vec<usize>>::new();
    let mut current_note_pages_by_marker = HashMap::<u16, HashSet<usize>>::new();
    for (position, candidate) in candidates.iter().copied().enumerate() {
        *candidate_counts_by_marker
            .entry(candidate.marker)
            .or_default() += 1;
        let line = &decoded[candidate.decoded_index].0;
        if decoded[candidate.decoded_index].1 == Lm2Action::Marginalia {
            current_note_pages_by_marker
                .entry(candidate.marker)
                .or_default()
                .insert(line.page_index);
            continue;
        }
        let auth_probability = auth_probabilities
            .get(candidate.decoded_index)
            .copied()
            .flatten()
            .map(|(_, probability)| probability)
            .unwrap_or(0.0);
        if auth_probability >= link_model.auth_threshold {
            candidate_positions_by_marker
                .entry(candidate.marker)
                .or_default()
                .push(position);
        }
    }
    let candidate_markers = candidate_positions_by_marker
        .keys()
        .copied()
        .collect::<HashSet<_>>();
    if candidate_markers.is_empty() {
        return 0;
    }
    let references = decoded
        .iter()
        .enumerate()
        .filter(|(_, (_, action))| *action == Lm2Action::Keep)
        .flat_map(|(index, (line, _))| {
            let leading_marker =
                lm2_note_head_normalized_text(&line.text).map(|(marker, _)| marker);
            let mut markers = sentineled_note_markers(&line.text)
                .into_iter()
                .filter(|marker| Some(*marker) != leading_marker)
                .collect::<HashSet<_>>();
            markers.extend(candidate_markers.iter().copied().filter(|marker| {
                Some(*marker) != leading_marker
                    && lm2_link_text_contains_marker(&line.text, *marker)
            }));
            markers.into_iter().map(move |marker| (index, marker))
        })
        .collect::<HashSet<_>>();
    let trace_path = std::env::var_os("LAWPDF_LM2_TRACE_LINK_RANKER_FILE").map(PathBuf::from);
    let mut trace_rows = Vec::new();
    let mut selected = HashSet::new();
    for (reference_index, marker) in references {
        let reference_page = decoded[reference_index].0.page_index;
        let already_resolved = current_note_pages_by_marker
            .get(&marker)
            .is_some_and(|pages| {
                pages.contains(&reference_page) || pages.contains(&(reference_page + 1))
            });
        if already_resolved {
            continue;
        }
        let local_positions = candidate_positions_by_marker
            .get(&marker)
            .into_iter()
            .flatten()
            .copied()
            .filter(|position| {
                let candidate = candidates[*position];
                let page = decoded[candidate.decoded_index].0.page_index;
                page == reference_page || page == reference_page + 1
            })
            .collect::<Vec<_>>();
        if local_positions.is_empty() {
            continue;
        }
        let document_same_marker_count = candidate_counts_by_marker
            .get(&marker)
            .copied()
            .unwrap_or(0);
        let mut scored = Vec::new();
        for position in &local_positions {
            let candidate = candidates[*position];
            let Some((_, auth_probability)) = auth_probabilities
                .get(candidate.decoded_index)
                .copied()
                .flatten()
            else {
                continue;
            };
            if auth_probability < link_model.auth_threshold {
                continue;
            }
            let features = lm2_link_ranker_feature_vector(
                decoded,
                &candidates,
                *position,
                reference_index,
                local_positions.len(),
                document_same_marker_count,
                auth_probability,
            );
            if features.len() != LM2_LINK_RANKER_FEATURE_COUNT_V1 {
                continue;
            }
            let link_probability = link_model.probability(&features);
            if trace_path.is_some() {
                let candidate_line = &decoded[candidate.decoded_index].0;
                let reference_line = &decoded[reference_index].0;
                trace_rows.push(serde_json::json!({
                    "document": document_path.display().to_string(),
                    "marker": marker,
                    "reference_id": reference_line.id.clone(),
                    "reference_page_index": reference_line.page_index,
                    "reference_line_index": reference_line.line_index,
                    "reference_text": reference_line.text.clone(),
                    "candidate_id": candidate_line.id.clone(),
                    "candidate_page_index": candidate_line.page_index,
                    "candidate_line_index": candidate_line.line_index,
                    "candidate_text": candidate_line.text.clone(),
                    "baseline_action": decoded[candidate.decoded_index].1.as_str(),
                    "auth_probability": auth_probability,
                    "auth_threshold": link_model.auth_threshold,
                    "link_probability": link_probability,
                    "link_threshold": link_model.threshold,
                    "features": features,
                }));
            }
            scored.push((candidate.decoded_index, auth_probability, link_probability));
        }
        scored.sort_by(|left, right| right.2.partial_cmp(&left.2).unwrap_or(Ordering::Equal));
        if let Some((candidate_index, auth_probability, link_probability)) = scored.first().copied()
            && auth_probability >= link_model.auth_threshold
            && link_probability >= link_model.threshold
        {
            selected.insert((candidate_index, marker));
        }
    }
    let changed = selected.len();
    for (index, marker) in selected {
        decoded[index].0.doc_note_marker = marker;
        decoded[index].1 = Lm2Action::Marginalia;
    }
    if let Some(path) = trace_path
        && !trace_rows.is_empty()
        && let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write as _;
        let mut writer = std::io::BufWriter::new(file);
        for row in trace_rows {
            if let Ok(encoded) = serde_json::to_string(&row) {
                let _ = writeln!(writer, "{encoded}");
            }
        }
        let _ = writer.flush();
    }
    changed
}

pub(super) fn apply_note_head_model(
    model: &Lm2NoteHeadModel,
    document_path: &Path,
    primary_emissions: Option<&[[f64; 3]]>,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let primary_emissions = primary_emissions.filter(|rows| rows.len() == decoded.len());
    let stacked = matches!(
        model.schema_version.as_str(),
        LM2_NOTE_HEAD_SCHEMA_V2 | LM2_NOTE_HEAD_SCHEMA_V3
    );
    if stacked && primary_emissions.is_none() {
        return 0;
    }
    let all_primary_probabilities = primary_emissions.map(|rows| {
        rows.iter()
            .copied()
            .map(lm2_context_primary_probabilities)
            .collect::<Vec<_>>()
    });
    let sequence_features = if model.schema_version == LM2_NOTE_HEAD_SCHEMA_V3 {
        all_primary_probabilities
            .as_deref()
            .map(|probabilities| lm2_note_head_sequence_features(decoded, probabilities))
    } else {
        None
    };
    let mut changed = 0usize;
    let trace = truthy_env("LAWPDF_LM2_TRACE_NOTE_HEAD");
    let trace_path = std::env::var_os("LAWPDF_LM2_TRACE_NOTE_HEAD_FILE").map(PathBuf::from);
    let mut trace_rows = Vec::new();
    for (index, (line, action)) in decoded.iter_mut().enumerate() {
        let primary_probabilities = if stacked {
            all_primary_probabilities
                .as_deref()
                .and_then(|rows| rows.get(index))
                .copied()
        } else {
            None
        };
        let Some((marker, mut features)) =
            lm2_note_head_feature_vector(line, primary_probabilities)
        else {
            if trace
                && (line.text.trim_start().starts_with(CALLOUT_START) || line.doc_note_marker > 0)
            {
                println!(
                    "note-head skipped page={} line={} marker={} action={} text={:?}",
                    line.page_index,
                    line.line_index,
                    line.doc_note_marker,
                    action.as_str(),
                    line.text
                );
            }
            continue;
        };
        if let Some(sequence) = sequence_features
            .as_ref()
            .and_then(|rows| rows.get(index))
            .copied()
            .flatten()
        {
            features.extend(sequence);
        }
        if features.len() != model.feature_count {
            continue;
        }
        let probability = model.probability(&features);
        if trace_path.is_some() {
            trace_rows.push(serde_json::json!({
                "document": document_path.display().to_string(),
                "id": line.id.clone(),
                "page_index": line.page_index,
                "line_index": line.line_index,
                "marker": marker,
                "probability": probability,
                "threshold": model.threshold,
                "primary_probabilities": primary_probabilities,
                "features": features,
                "baseline_action": action.as_str(),
                "text": line.text.clone(),
            }));
        }
        if trace {
            eprintln!(
                "note-head p={probability:.6} threshold={:.6} primary={primary_probabilities:?} page={} line={} action={} text={:?}",
                model.threshold,
                line.page_index,
                line.line_index,
                action.as_str(),
                line.text
            );
        }
        if probability < model.threshold {
            continue;
        }
        line.doc_note_marker = marker;
        if *action != Lm2Action::Marginalia {
            *action = Lm2Action::Marginalia;
            changed += 1;
        }
    }
    if let Some(path) = trace_path
        && !trace_rows.is_empty()
        && let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write as _;
        let mut writer = std::io::BufWriter::new(file);
        for row in trace_rows {
            if let Ok(encoded) = serde_json::to_string(&row) {
                let _ = writeln!(writer, "{encoded}");
            }
        }
        let _ = writer.flush();
    }
    changed
}

pub(super) fn lm2_context_arbiter_policy(
    baseline: Lm2Action,
    arbiter: [f64; 3],
    calibration: &Lm2ContextArbiterCalibration,
) -> Lm2Action {
    if baseline != Lm2Action::Keep {
        if arbiter[1] >= calibration.rescue_keep_threshold {
            return Lm2Action::Keep;
        }
        return if arbiter[0] >= arbiter[2] {
            if arbiter[0] >= calibration.reclassify_nonkeep_threshold {
                Lm2Action::HideNoise
            } else {
                baseline
            }
        } else if arbiter[2] >= calibration.reclassify_nonkeep_threshold {
            Lm2Action::Marginalia
        } else {
            baseline
        };
    }

    let mut prediction = Lm2Action::Keep;
    if arbiter[2] >= calibration.demote_to_marginalia_threshold {
        prediction = Lm2Action::Marginalia;
    }
    if arbiter[0] >= calibration.demote_to_noise_threshold {
        prediction = Lm2Action::HideNoise;
    }
    prediction
}

pub(super) fn lm2_context_runtime_residual_policy(
    baseline: Lm2Action,
    residual: [f64; 3],
    calibration: &Lm2ContextArbiterCalibration,
    text: &str,
) -> Lm2Action {
    if baseline != Lm2Action::Keep
        && residual[1] >= calibration.rescue_keep_threshold
        && lm2_context_has_substantive_text(text)
    {
        Lm2Action::Keep
    } else {
        baseline
    }
}

pub(super) fn lm2_context_norm_path(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

pub(super) fn lm2_context_action_index(action: Lm2Action) -> usize {
    match action {
        Lm2Action::HideNoise => 0,
        Lm2Action::Keep => 1,
        Lm2Action::Marginalia => 2,
    }
}

pub(super) fn lm2_context_onehot(features: &mut Vec<f64>, action: Option<Lm2Action>) {
    let selected = action.map(lm2_context_action_index);
    for index in 0..3 {
        features.push(if selected == Some(index) { 1.0 } else { 0.0 });
    }
}

pub(super) fn lm2_context_block_meta(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashMap<String, Lm2ContextBlockMeta> {
    let page_count = decoded
        .iter()
        .map(|(line, _)| line.page_index)
        .max()
        .map_or(0, |page| page + 1);
    let article_spans = detect_lm2_article_spans(decoded, page_count);
    let (_, _, block_source_lines) =
        build_lm2_blocks_with_grouping("", decoded, None, &article_spans);
    let lines_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.clone(), line))
        .collect::<HashMap<_, _>>();
    let mut out = HashMap::new();
    for block in block_source_lines {
        let full_rows = block
            .lines
            .iter()
            .filter_map(|line| line.id.as_ref().and_then(|id| lines_by_id.get(id).copied()))
            .collect::<Vec<_>>();
        if full_rows.is_empty() {
            continue;
        }
        let n = full_rows.len().max(1) as f64;
        let block_text = full_rows
            .iter()
            .map(|line| collapse_whitespace(&line.text))
            .collect::<Vec<_>>()
            .join(" ");
        let shapes = full_rows
            .iter()
            .map(|line| lm2_context_text_shape(&collapse_whitespace(&line.text)))
            .collect::<Vec<_>>();
        let block_action = block
            .lines
            .iter()
            .map(|line| lm2_context_action_for_role(line.role))
            .collect::<Vec<_>>();
        let block_action = most_common_lm2_context_action(&block_action);
        let meta = Lm2ContextBlockMeta {
            block_action,
            block_line_count: full_rows.len(),
            block_char_count: block_text.chars().count(),
            block_short_ratio: shapes.iter().map(|shape| shape.short_text).sum::<f64>() / n,
            block_numeric_ratio: shapes
                .iter()
                .map(|shape| shape.numeric_or_roman)
                .sum::<f64>()
                / n,
            block_dotleader: if shapes.iter().any(|shape| shape.dotleader > 0.0) {
                1.0
            } else {
                0.0
            },
            block_note_start_ratio: shapes.iter().map(|shape| shape.note_start).sum::<f64>() / n,
            block_edge_ratio: full_rows
                .iter()
                .filter(|line| {
                    line.doc_repeated_top_edge
                        || line.doc_repeated_bottom_edge
                        || line.doc_repeated_edge_text
                })
                .count() as f64
                / n,
            block_axis_ratio: full_rows
                .iter()
                .filter(|line| {
                    line.doc_vertical_axis_like
                        || line.doc_vertical_numeric_axis_like
                        || line.page_table_column_like
                })
                .count() as f64
                / n,
            block_footzone_ratio: full_rows
                .iter()
                .filter(|line| line.in_footnote_zone || line.below_footnote_divider)
                .count() as f64
                / n,
            block_pos_norm: 0.0,
        };
        let denom = full_rows.len().saturating_sub(1).max(1) as f64;
        for (position, line) in full_rows.iter().enumerate() {
            let mut line_meta = meta.clone();
            line_meta.block_pos_norm = position as f64 / denom;
            out.insert(line.id.clone(), line_meta);
        }
    }
    out
}

pub(super) fn most_common_lm2_context_action(actions: &[Lm2Action]) -> Option<Lm2Action> {
    ACTIONS
        .into_iter()
        .max_by_key(|candidate| actions.iter().filter(|action| *action == candidate).count())
}

pub(super) fn lm2_context_action_for_role(role: LiquidBlockRole) -> Lm2Action {
    match role {
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia => Lm2Action::Marginalia,
        LiquidBlockRole::Table
        | LiquidBlockRole::Caption
        | LiquidBlockRole::Contents
        | LiquidBlockRole::Header
        | LiquidBlockRole::Footer
        | LiquidBlockRole::Metadata
        | LiquidBlockRole::SectionBreak
        | LiquidBlockRole::Noise => Lm2Action::HideNoise,
        _ => Lm2Action::Keep,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Lm2ContextTextShape {
    pub(super) char_count: f64,
    pub(super) token_count: f64,
    pub(super) short_text: f64,
    pub(super) numeric_or_roman: f64,
    pub(super) allcaps: f64,
    pub(super) digit_ratio: f64,
    pub(super) upper_ratio: f64,
    pub(super) dotleader: f64,
    pub(super) note_start: f64,
}

pub(super) fn lm2_context_text_shape(text: &str) -> Lm2ContextTextShape {
    let chars = text.chars().count();
    let mut token_count = 0usize;
    let mut in_token = false;
    let mut ascii_letters = Vec::new();
    let mut digit_count = 0usize;
    for ch in text.chars() {
        let is_token = ch.is_ascii_alphanumeric() || ch == '§';
        if is_token && !in_token {
            token_count += 1;
        }
        in_token = is_token;
        if ch.is_ascii_alphabetic() {
            ascii_letters.push(ch);
        }
        if ch.is_ascii_digit() {
            digit_count += 1;
        }
    }
    let letter_count = ascii_letters.len();
    let upper_count = ascii_letters
        .iter()
        .filter(|ch| ch.is_ascii_uppercase())
        .count();
    let numeric_or_roman = !text.is_empty()
        && text.chars().count() <= 8
        && text.chars().all(|ch| {
            ch.is_ascii_digit()
                || matches!(
                    ch,
                    'I' | 'V'
                        | 'X'
                        | 'L'
                        | 'C'
                        | 'D'
                        | 'M'
                        | 'i'
                        | 'v'
                        | 'x'
                        | 'l'
                        | 'c'
                        | 'd'
                        | 'm'
                )
        });
    let allcaps = letter_count >= 8 && upper_count == letter_count;
    Lm2ContextTextShape {
        char_count: chars as f64,
        token_count: token_count as f64,
        short_text: if token_count <= 4 || chars <= 24 {
            1.0
        } else {
            0.0
        },
        numeric_or_roman: if numeric_or_roman { 1.0 } else { 0.0 },
        allcaps: if allcaps { 1.0 } else { 0.0 },
        digit_ratio: digit_count as f64 / chars.max(1) as f64,
        upper_ratio: upper_count as f64 / letter_count.max(1) as f64,
        dotleader: if lm2_context_has_dotleader(text) {
            1.0
        } else {
            0.0
        },
        note_start: if lm2_context_note_start(text) {
            1.0
        } else {
            0.0
        },
    }
}

pub(super) fn lm2_context_has_dotleader(text: &str) -> bool {
    text.contains("...") || text.contains("···") || text.contains('…')
}

pub(super) fn lm2_context_note_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if matches!(first, '*' | '†' | '‡' | '§') {
        return chars.next().is_some_and(char::is_whitespace);
    }
    if first.is_ascii_lowercase() {
        return chars.next() == Some(')') && chars.next().is_some_and(char::is_whitespace);
    }
    let mut rest = trimmed;
    if rest.starts_with('(') {
        rest = &rest[1..];
    }
    let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digit_count) {
        return false;
    }
    rest = &rest[digit_count..];
    if rest.starts_with(')') {
        rest = &rest[1..];
    }
    rest.chars().next().is_some_and(char::is_whitespace)
}

pub(super) fn lm2_context_feature_vector(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    baseline_actions: &[Lm2Action],
    block_meta: &HashMap<String, Lm2ContextBlockMeta>,
    index: usize,
) -> Vec<f64> {
    let line = &decoded[index].0;
    let shape = lm2_context_text_shape(&collapse_whitespace(&line.text));
    let block = block_meta.get(&line.id).cloned().unwrap_or_default();
    let width = (line.page_width as f64).max(1.0);
    let height = (line.page_height as f64).max(1.0);
    let mut features = Vec::with_capacity(66);
    lm2_context_onehot(&mut features, baseline_actions.get(index).copied());
    lm2_context_onehot(
        &mut features,
        index
            .checked_sub(1)
            .and_then(|idx| baseline_actions.get(idx))
            .copied(),
    );
    lm2_context_onehot(&mut features, baseline_actions.get(index + 1).copied());
    lm2_context_onehot(
        &mut features,
        index
            .checked_sub(2)
            .and_then(|idx| baseline_actions.get(idx))
            .copied(),
    );
    lm2_context_onehot(&mut features, baseline_actions.get(index + 2).copied());
    features.extend([
        line.left as f64 / width,
        line.right as f64 / width,
        line.top as f64 / height,
        line.bottom as f64 / height,
        line.font_height as f64,
        line.font_ratio_page as f64,
        line.font_ratio_doc as f64,
        line.doc_font_body_z as f64,
        line.doc_font_footnote_z as f64,
        bool_as_f64(line.bold),
        bool_as_f64(line.italic),
        bool_as_f64(line.centered),
        bool_as_f64(line.below_footnote_divider),
        bool_as_f64(line.in_footnote_zone),
        bool_as_f64(
            line.doc_repeated_top_edge
                || line.doc_repeated_bottom_edge
                || line.doc_repeated_edge_text,
        ),
        bool_as_f64(
            line.doc_vertical_axis_like
                || line.doc_vertical_numeric_axis_like
                || line.page_table_column_like,
        ),
        bool_as_f64(line.prev_line_has_dotleader || line.prev4_toc_leader_context),
        line.doc_note_marker as f64,
        bool_as_f64(line.doc_note_marker_first_on_page),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
        shape.char_count,
        shape.token_count,
        shape.short_text,
        shape.numeric_or_roman,
        shape.allcaps,
        shape.digit_ratio,
        shape.upper_ratio,
        shape.dotleader,
        shape.note_start,
        block.block_line_count.max(1) as f64,
        block.block_char_count as f64,
        block.block_short_ratio,
        block.block_numeric_ratio,
        block.block_dotleader,
        block.block_note_start_ratio,
        block.block_edge_ratio,
        block.block_axis_ratio,
        block.block_footzone_ratio,
        block.block_pos_norm,
        bool_as_f64(line.segment_block_first),
        bool_as_f64(line.segment_block_last),
        line.segment_block_line_count.max(1) as f64,
        line.segment_block_line_index as f64,
        bool_as_f64(line.segment_block_footnote_like),
        bool_as_f64(line.segment_block_furniture_like),
        bool_as_f64(line.segment_block_table_like),
        bool_as_f64(line.segment_block_toc_like),
    ]);
    lm2_context_onehot(
        &mut features,
        block
            .block_action
            .or_else(|| baseline_actions.get(index).copied()),
    );
    features
}

impl Lm2Model {
    pub(super) fn doc_font_zscores_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.doc_font_zscores)
    }

    pub(super) fn repetition_fingerprints_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.repetition_fingerprints)
    }

    pub(super) fn marker_continuity_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.marker_continuity)
    }
}

impl Lm2NativeCatboostModel {
    pub(super) fn emission_scores(&self, line: &DeepLiquidSourceLine) -> Result<[f64; 3], String> {
        let feature_map = lm2_numeric_catboost_features(line);
        let float_features = LM2_NATIVE_CATBOOST_FLOAT_FEATURES
            .iter()
            .map(|name| feature_map.get(*name).copied().unwrap_or(0.0) as c_float)
            .collect::<Vec<_>>();
        let cat_values = lm2_native_catboost_cat_features(line);
        let cat_cstrings = cat_values
            .iter()
            .map(|value| {
                CString::new(value.as_str())
                    .map_err(|_| "NUL byte in categorical feature".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let text = collapse_whitespace(&line.text)
            .chars()
            .take(500)
            .collect::<String>();
        let text_cstring =
            CString::new(text).map_err(|_| "NUL byte in CatBoost text feature".to_owned())?;

        let float_rows = [float_features.as_ptr()];
        let cat_ptrs = cat_cstrings
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        let cat_rows = [cat_ptrs.as_ptr()];
        let text_ptrs = [text_cstring.as_ptr()];
        let text_rows = [text_ptrs.as_ptr()];
        let mut raw = vec![0.0; self.dimensions_count];
        let ok = unsafe {
            (self.calc_model_prediction_text)(
                self.handle,
                1,
                float_rows.as_ptr(),
                float_features.len(),
                cat_rows.as_ptr(),
                cat_ptrs.len(),
                text_rows.as_ptr(),
                text_ptrs.len(),
                raw.as_mut_ptr(),
                raw.len(),
            )
        };
        if !ok {
            return Err(self.last_error());
        }
        if raw.len() != 3 {
            return Err(format!("native CatBoost returned {} dimensions", raw.len()));
        }
        // This promoted CBM reports classes in CatBoost/Python order:
        // ["hide_noise", "keep", "marginalia"].
        Ok([
            raw[1], // keep
            raw[2], // marginalia
            raw[0], // hide_noise
        ])
    }

    pub(super) fn last_error(&self) -> String {
        let ptr = unsafe { (self.get_error_string)() };
        if ptr.is_null() {
            return "CatBoost C API error".to_owned();
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Lm2NativeCatboostModel {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                (self.delete_model)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

impl Lm2NumericCatboostModel {
    pub(super) fn is_usable(&self) -> bool {
        self.schema_version == "lm2-numeric-catboost-runtime-v1"
            && self.model_type == "numeric_catboost_oblivious_trees_v1"
            && self.classes.len() == ACTIONS.len()
            && self.bias.len() == self.classes.len()
            && !self.features.is_empty()
            && !self.trees.is_empty()
            && self.trees.iter().all(|tree| {
                let leaf_count = 1usize << tree.splits.len();
                tree.leaf_values.len() == leaf_count * self.classes.len()
                    && tree
                        .splits
                        .iter()
                        .all(|split| split.feature_index < self.features.len())
            })
    }

    pub(super) fn emission_scores(&self, line: &DeepLiquidSourceLine) -> [f64; 3] {
        let features = lm2_numeric_catboost_features(line);
        let mut class_scores = self.bias.clone();
        class_scores.resize(self.classes.len(), 0.0);
        for tree in &self.trees {
            let mut leaf = 0usize;
            for (depth, split) in tree.splits.iter().enumerate() {
                let feature_name = &self.features[split.feature_index].name;
                let value = features.get(feature_name.as_str()).copied().unwrap_or(0.0);
                if value > split.border {
                    leaf |= 1usize << depth;
                }
            }
            let offset = leaf * self.classes.len();
            for (class_index, score) in class_scores.iter_mut().enumerate() {
                *score += self.scale * tree.leaf_values[offset + class_index];
            }
        }
        let mut action_scores = [0.0, 0.0, 0.0];
        for (class_index, class_name) in self.classes.iter().enumerate() {
            let action_index = match class_name.as_str() {
                "keep" => Lm2Action::Keep.index(),
                "marginalia" => Lm2Action::Marginalia.index(),
                "hide_noise" => Lm2Action::HideNoise.index(),
                _ => continue,
            };
            action_scores[action_index] = class_scores[class_index];
        }
        action_scores
    }
}

pub(super) fn model_is_usable(model: &Lm2Model) -> bool {
    model.model_type == "hashed_softmax_action_v1"
        && model.actions == ACTIONS.map(|action| action.as_str().to_owned())
        && model.feature_dim > 0
        && model.bias.len() == ACTIONS.len()
        && model.weights.len() == ACTIONS.len()
        && model
            .weights
            .iter()
            .all(|weights| weights.len() == model.feature_dim)
}

pub(super) fn load_lm2_model() -> Result<Lm2Model, String> {
    let candidates = lm2_model_candidates();
    let path = candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| missing_model_error("legacy LM2 model", &candidates))?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("Could not read LM2 model: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Could not decode LM2 model: {error}"))
}

pub(super) fn load_lm2_native_catboost_model() -> Result<Option<Lm2NativeCatboostModel>, String> {
    let mut model_candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_MODEL").map(PathBuf::from) {
        model_candidates.push(path);
    }
    model_candidates.extend(lm2_native_catboost_runtime_asset_candidates(
        LM2_NATIVE_CATBOOST_MODEL_FILE,
    ));
    let model_path = model_candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| missing_model_error("LM2 native CatBoost model", &model_candidates))?;
    verify_model_asset_hash(&model_path, "native_model")?;
    let model_sha256 = sha256_hex_of_file(&model_path)?;

    let mut library_candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_LIB").map(PathBuf::from) {
        library_candidates.push(path);
    }
    library_candidates.extend(lm2_native_catboost_runtime_asset_candidates(
        lm2_native_catboost_library_file(),
    ));
    let lib_path = library_candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| missing_model_error("LM2 native CatBoost library", &library_candidates))?;
    let library = unsafe { Library::new(&lib_path) }
        .map_err(|error| format!("Could not load native CatBoost library: {error}"))?;
    let (
        create_model,
        delete_model,
        load_full_model_from_file,
        get_float_features_count,
        get_cat_features_count,
        get_text_features_count,
        get_dimensions_count,
        get_error_string,
        calc_model_prediction_text,
    ) = unsafe {
        (
            *library
                .get::<CatboostCreateFn>(b"ModelCalcerCreate\0")
                .map_err(|error| format!("CatBoost library missing ModelCalcerCreate: {error}"))?,
            *library
                .get::<CatboostDeleteFn>(b"ModelCalcerDelete\0")
                .map_err(|error| format!("CatBoost library missing ModelCalcerDelete: {error}"))?,
            *library
                .get::<CatboostLoadFullModelFromFileFn>(b"LoadFullModelFromFile\0")
                .map_err(|error| {
                    format!("CatBoost library missing LoadFullModelFromFile: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetFloatFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetFloatFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetCatFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetCatFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetTextFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetTextFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetDimensionsCount\0")
                .map_err(|error| format!("CatBoost library missing GetDimensionsCount: {error}"))?,
            *library
                .get::<CatboostGetErrorStringFn>(b"GetErrorString\0")
                .map_err(|error| format!("CatBoost library missing GetErrorString: {error}"))?,
            *library
                .get::<CatboostCalcModelPredictionTextFn>(b"CalcModelPredictionText\0")
                .map_err(|error| {
                    format!("CatBoost library missing CalcModelPredictionText: {error}")
                })?,
        )
    };

    let handle = unsafe { create_model() };
    if handle.is_null() {
        return Err("Could not create CatBoost model handle".to_owned());
    }
    let model_path_string = model_path.to_string_lossy().into_owned();
    let model_path_c = CString::new(model_path_string)
        .map_err(|_| "NUL byte in LAWPDF_LM2_NATIVE_CATBOOST_MODEL".to_owned())?;
    let loaded = unsafe { load_full_model_from_file(handle, model_path_c.as_ptr()) };
    if !loaded {
        let error = catboost_error_string(get_error_string);
        unsafe {
            delete_model(handle);
        }
        return Err(format!(
            "Could not load native CatBoost model {}: {error}",
            model_path.display()
        ));
    }
    let float_feature_count = unsafe { get_float_features_count(handle) };
    let cat_feature_count = unsafe { get_cat_features_count(handle) };
    let text_feature_count = unsafe { get_text_features_count(handle) };
    let dimensions_count = unsafe { get_dimensions_count(handle) };
    if float_feature_count != LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len()
        || cat_feature_count != LM2_NATIVE_CATBOOST_CAT_FEATURES.len()
        || text_feature_count != 1
        || dimensions_count != 3
    {
        unsafe {
            delete_model(handle);
        }
        return Err(format!(
            "Native CatBoost feature contract mismatch: got f{float_feature_count}/c{cat_feature_count}/t{text_feature_count}/d{dimensions_count}, expected f{}/c{}/t1/d3",
            LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len(),
            LM2_NATIVE_CATBOOST_CAT_FEATURES.len()
        ));
    }
    Ok(Some(Lm2NativeCatboostModel {
        _library: library,
        handle,
        delete_model,
        calc_model_prediction_text,
        get_error_string,
        float_feature_count,
        cat_feature_count,
        text_feature_count,
        dimensions_count,
        model_sha256,
    }))
}

pub(super) fn catboost_error_string(get_error_string: CatboostGetErrorStringFn) -> String {
    let ptr = unsafe { get_error_string() };
    if ptr.is_null() {
        return "CatBoost C API error".to_owned();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

pub(super) fn load_lm2_numeric_catboost_model() -> Result<Option<Lm2NumericCatboostModel>, String> {
    let path = std::env::var_os("LAWPDF_LM2_NUMERIC_CATBOOST_MODEL")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("lm2-numeric-catboost-runtime.json"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    let Some(path) = path else { return Ok(None) };
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Could not read LM2 numeric CatBoost model: {error}"))?;
    let model = serde_json::from_slice::<Lm2NumericCatboostModel>(&bytes)
        .map_err(|error| format!("Could not decode LM2 numeric CatBoost model: {error}"))?;
    if !model.is_usable() {
        return Err("LM2 numeric CatBoost model is not usable".to_owned());
    }
    Ok(Some(model))
}

pub(super) fn load_lm2_static_front_overlay() -> Result<Option<Lm2StaticFrontOverlay>, String> {
    let a55_path = std::env::var_os("LAWPDF_LM2_A55_OVERLAY")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("a55-front-stack-overlay.jsonl"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    let d3_path = std::env::var_os("LAWPDF_LM2_D3_OVERLAY")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("d3-front-matter-regions.jsonl"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    if a55_path.is_none() && d3_path.is_none() {
        return Ok(None);
    }

    let mut roles_by_doc_line: HashMap<String, HashMap<String, LiquidBlockRole>> = HashMap::new();
    let mut labels = Vec::new();
    if let Some(path) = a55_path {
        load_lm2_a55_overlay_rows(&path, &mut roles_by_doc_line)?;
        labels.push(format!("a55:{:016x}", fnv1a64(&path.display().to_string())));
    }
    if let Some(path) = d3_path {
        load_lm2_d3_overlay_rows(&path, &mut roles_by_doc_line)?;
        labels.push(format!("d3:{:016x}", fnv1a64(&path.display().to_string())));
    }
    if roles_by_doc_line.is_empty() {
        return Ok(None);
    }
    Ok(Some(Lm2StaticFrontOverlay {
        source_label: labels.join("+"),
        roles_by_doc_line,
    }))
}

pub(super) fn load_lm2_a55_overlay_rows(
    path: &Path,
    roles_by_doc_line: &mut HashMap<String, HashMap<String, LiquidBlockRole>>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read LM2 A55 overlay rows: {error}"))?;
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Could not decode LM2 A55 overlay row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let doc = normalize_path_key_value(row.get("doc_path"));
        let line_id = row
            .get("line_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let role = row
            .get("to_role")
            .and_then(|value| value.as_str())
            .and_then(role_from_name);
        if doc.is_empty() || line_id.is_empty() {
            continue;
        }
        if let Some(role) = role {
            roles_by_doc_line
                .entry(doc)
                .or_default()
                .insert(line_id, role);
        }
    }
    Ok(())
}

pub(super) fn load_lm2_d3_overlay_rows(
    path: &Path,
    roles_by_doc_line: &mut HashMap<String, HashMap<String, LiquidBlockRole>>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read LM2 D3 overlay rows: {error}"))?;
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Could not decode LM2 D3 overlay row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let doc = normalize_path_key_value(row.get("source_path"));
        let line_id = row
            .get("line_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let role = match row
            .get("front_matter_kind")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
        {
            "title" => Some(LiquidBlockRole::Title),
            "section_heading" => Some(LiquidBlockRole::Heading),
            "author" | "abstract" => Some(LiquidBlockRole::Marginalia),
            _ => None,
        };
        if doc.is_empty() || line_id.is_empty() {
            continue;
        }
        if let Some(role) = role {
            roles_by_doc_line
                .entry(doc)
                .or_default()
                .insert(line_id, role);
        }
    }
    Ok(())
}

pub(super) fn normalize_path_key_value(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .replace('\\', "/")
        .to_lowercase()
}

pub(super) fn load_lm2_pp_priors() -> Result<Option<Lm2PpPriorIndex>, String> {
    let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DRAFTS").map(PathBuf::from) else {
        return Ok(None);
    };
    load_lm2_pp_priors_from_path(path)
}

pub(super) fn load_lm2_pp_priors_from_path(
    path: PathBuf,
) -> Result<Option<Lm2PpPriorIndex>, String> {
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read LM2 PP draft rows: {error}"))?;
    let mut rows = HashMap::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Lm2PpDraftRow = serde_json::from_str(line).map_err(|error| {
            format!(
                "Could not decode LM2 PP draft row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let action = row.draft_action.as_deref().or(row.pp_action.as_deref());
        let role = row.pp_role.unwrap_or_default();
        let label = row.pp_label.unwrap_or_default();
        let score = row.pp_score.unwrap_or_default();
        if !guarded_pp_prior_action(action, &role, &label, score, &row.text) {
            continue;
        }
        let source_path = if row.source_path.is_empty() {
            row.path
        } else {
            row.source_path
        };
        if source_path.is_empty() {
            continue;
        }
        rows.insert(
            pp_prior_key(&source_path, row.page_index, row.line_index, &row.text),
            Lm2PpPrior { role, label, score },
        );
    }
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(Lm2PpPriorIndex { source: path, rows }))
}

pub(super) fn load_or_generate_lm2_pp_priors(
    path: &Path,
    source_lines: &[DeepLiquidSourceLine],
) -> Result<Option<Lm2PpPriorIndex>, String> {
    if source_lines.is_empty() {
        return Ok(None);
    }
    let cache_key = lm2_pp_doclayout_cache_key(path, source_lines);
    let draft_path = lm2_pp_doclayout_draft_cache_path(&cache_key)
        .ok_or_else(|| "could not find LM2 PP-DocLayout cache directory".to_owned())?;
    if draft_path.is_file() {
        return load_lm2_pp_priors_from_path(draft_path);
    }
    run_lm2_pp_doclayout_sidecar(path, source_lines, &cache_key, &draft_path)?;
    if draft_path.is_file() {
        return load_lm2_pp_priors_from_path(draft_path);
    }
    Ok(None)
}

pub(super) fn lm2_pp_prior_index_has_footnotes(index: &Lm2PpPriorIndex) -> bool {
    index
        .rows
        .values()
        .any(|prior| prior.role == "footnote" && prior.score >= 0.80)
}

pub(super) fn guarded_pp_prior_action(
    action: Option<&str>,
    role: &str,
    label: &str,
    score: f64,
    text: &str,
) -> bool {
    let compact_text = text.split_whitespace().collect::<String>();
    role == "footnote"
        && action == Some("marginalia")
        && score >= 0.80
        && !compact_text.chars().all(|ch| ch.is_ascii_digit())
        || role == "table" && action == Some("hide_noise") && score >= 0.70
        || label == "number" && action == Some("hide_noise") && score >= 0.80
}

pub(super) fn lm2_model_candidates() -> Vec<PathBuf> {
    lm2_runtime_asset_candidates("profile-models/lm2-current", "lm2-model.json")
}

pub(super) fn lm2_v20_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    lm2_runtime_asset_candidates("profile-models/lm2-v20-runtime", file_name)
}

pub(super) fn lm2_native_catboost_library_file() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libcatboostmodel-darwin-universal2-1.2.10.dylib"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "libcatboostmodel-linux-x86_64-1.2.10.so"
    }
    #[cfg(target_os = "windows")]
    {
        "catboostmodel.dll"
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "linux", target_arch = "x86_64"),
        target_os = "windows"
    )))]
    {
        "libcatboostmodel"
    }
}

pub(crate) fn lm2_native_catboost_default_asset_available() -> bool {
    lm2_native_catboost_runtime_asset_candidates(LM2_NATIVE_CATBOOST_MODEL_FILE)
        .into_iter()
        .any(|path| path.is_file())
        && lm2_native_catboost_runtime_asset_candidates(lm2_native_catboost_library_file())
            .into_iter()
            .any(|path| path.is_file())
}

pub(crate) fn lm2_fasttab_default_asset_available() -> bool {
    lm2_fasttab_runtime_asset_candidates(LM2_FASTTAB_MODEL_FILE)
        .into_iter()
        .any(|path| path.is_file())
}

pub(crate) fn lm2_native_line_default_asset_available() -> bool {
    lm2_native_catboost_default_asset_available()
        || (fasttab_enabled() && lm2_fasttab_default_asset_available())
}

pub(super) fn lm2_context_twopass_enabled() -> bool {
    !falsey_env("LAWPDF_LM2_CONTEXT_TWOPASS")
}

pub(super) fn load_lm2_context_twopass_model(
    active_primary_model_sha256: Option<&str>,
) -> Result<Option<Lm2ContextTwopassModel>, String> {
    let candidates = lm2_context_twopass_runtime_asset_candidates(LM2_CONTEXT_TWOPASS_MODEL_FILE);
    let path = candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| missing_model_error("LM2 context two-pass model", &candidates))?;
    load_lm2_context_model_from_path(&path, "context_model", active_primary_model_sha256, None)
        .map(Some)
}

pub(super) fn load_lm2_context_arbiter_model(
    active_primary_model_sha256: Option<&str>,
    active_baseline_context_model_sha256: Option<&str>,
) -> Result<Option<Lm2ContextTwopassModel>, String> {
    let candidates = lm2_context_arbiter_runtime_asset_candidates(LM2_CONTEXT_ARBITER_MODEL_FILE);
    let Some(path) = candidates.iter().find(|path| path.is_file()).cloned() else {
        return Ok(None);
    };
    let model = load_lm2_context_model_from_path(
        &path,
        "context_arbiter_model",
        active_primary_model_sha256,
        active_baseline_context_model_sha256,
    )?;
    if !matches!(
        model.schema_version.as_str(),
        LM2_CONTEXT_ARBITER_SCHEMA_V2 | LM2_CONTEXT_RESIDUAL_SCHEMA_V3
    ) {
        return Err(format!(
            "LM2 context arbiter must use schema {LM2_CONTEXT_ARBITER_SCHEMA_V2} or {LM2_CONTEXT_RESIDUAL_SCHEMA_V3}, found {}",
            model.schema_version
        ));
    }
    Ok(Some(model))
}

pub(super) fn load_lm2_context_model_from_path(
    path: &Path,
    release_asset_name: &str,
    active_primary_model_sha256: Option<&str>,
    active_baseline_context_model_sha256: Option<&str>,
) -> Result<Lm2ContextTwopassModel, String> {
    verify_model_asset_hash(path, release_asset_name)?;
    let asset_sha256 = sha256_hex_of_file(&path)?;
    let input: Lm2ContextTwopassModelFile = read_json_file(&path)?;
    if input.actions != ["hide_noise", "keep", "marginalia"] {
        return Err(format!(
            "LM2 context two-pass model has unexpected action order {:?}",
            input.actions
        ));
    }
    match input.schema_version.as_str() {
        "lawpdf-lm2-context-twopass-hgb-v1" => {
            if input.feature_count != 66 {
                return Err(format!(
                    "LM2 context two-pass v1 model has unexpected feature count {}",
                    input.feature_count
                ));
            }
        }
        LM2_CONTEXT_ARBITER_SCHEMA_V2 | LM2_CONTEXT_RESIDUAL_SCHEMA_V3 => {
            let is_residual = input.schema_version == LM2_CONTEXT_RESIDUAL_SCHEMA_V3;
            let expected_feature_count = if is_residual {
                LM2_CONTEXT_RESIDUAL_FEATURE_COUNT_V3
            } else {
                LM2_CONTEXT_ARBITER_FEATURE_COUNT_V2
            };
            if input.feature_count != expected_feature_count
                || input.numeric_feature_count != Some(LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len())
                || input.primary_probability_order != ["hide_noise", "keep", "marginalia"]
                || input.neighbor_offsets != LM2_CONTEXT_ARBITER_NEIGHBOR_OFFSETS
                || (is_residual
                    && input.baseline_action_order != ["hide_noise", "keep", "marginalia"])
            {
                return Err(format!(
                    "LM2 context arbiter feature contract mismatch: schema={}/f{}/numeric{:?}/probability_order={:?}/offsets={:?}/baseline_order={:?}",
                    input.schema_version,
                    input.feature_count,
                    input.numeric_feature_count,
                    input.primary_probability_order,
                    input.neighbor_offsets,
                    input.baseline_action_order
                ));
            }
            let expected_primary = input.primary_model_sha256.as_deref().ok_or_else(|| {
                "LM2 context arbiter v2 is missing primary_model_sha256".to_owned()
            })?;
            let active_primary = active_primary_model_sha256.ok_or_else(|| {
                "LM2 context arbiter v2 requires its exact CatBoost primary; it cannot consume FastTab or fallback emissions"
                    .to_owned()
            })?;
            if !expected_primary.eq_ignore_ascii_case(active_primary) {
                return Err(format!(
                    "LM2 context arbiter v2 primary-model mismatch: expected {expected_primary}, active {active_primary}"
                ));
            }
            if is_residual {
                let expected_context =
                    input
                        .baseline_context_model_sha256
                        .as_deref()
                        .ok_or_else(|| {
                            "LM2 runtime residual v3 is missing baseline_context_model_sha256"
                                .to_owned()
                        })?;
                let active_context = active_baseline_context_model_sha256.ok_or_else(|| {
                    "LM2 runtime residual v3 requires its exact baseline context model".to_owned()
                })?;
                if !expected_context.eq_ignore_ascii_case(active_context) {
                    return Err(format!(
                        "LM2 runtime residual v3 baseline-context mismatch: expected {expected_context}, active {active_context}"
                    ));
                }
            }
            let calibration = input.calibration.as_ref().ok_or_else(|| {
                "LM2 context arbiter v2 is missing calibrated policy thresholds".to_owned()
            })?;
            for (name, value) in [
                ("rescue_keep_threshold", calibration.rescue_keep_threshold),
                (
                    "demote_to_marginalia_threshold",
                    calibration.demote_to_marginalia_threshold,
                ),
                (
                    "demote_to_noise_threshold",
                    calibration.demote_to_noise_threshold,
                ),
                (
                    "reclassify_nonkeep_threshold",
                    calibration.reclassify_nonkeep_threshold,
                ),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!(
                        "LM2 context arbiter v2 has invalid {name}: {value}"
                    ));
                }
            }
            if is_residual
                && (calibration.demote_to_marginalia_threshold != 1.0
                    || calibration.demote_to_noise_threshold != 1.0
                    || calibration.reclassify_nonkeep_threshold != 1.0)
            {
                return Err(
                    "LM2 runtime residual v3 must be rescue-only; all demotion and nonkeep-reclassification thresholds must be closed at 1.0"
                        .to_owned(),
                );
            }
        }
        schema => {
            return Err(format!(
                "LM2 context two-pass model has unsupported schema {schema}"
            ));
        }
    }
    if input.models.is_empty()
        || input.models.iter().any(|model| {
            model.baseline_prediction.len() != 3
                || model.trees.iter().any(|iteration| iteration.len() != 3)
        })
    {
        return Err("LM2 context model has an invalid three-class HGB structure".to_owned());
    }
    Ok(Lm2ContextTwopassModel {
        schema_version: input.schema_version,
        actions: input.actions,
        feature_count: input.feature_count,
        calibration: input.calibration,
        primary_model_sha256: input.primary_model_sha256,
        asset_sha256,
        doc_to_fold: input
            .doc_to_fold
            .into_iter()
            .map(|(path, fold)| (lm2_context_norm_path(&path), fold))
            .collect(),
        unseen_doc_model: input.unseen_doc_model.unwrap_or_else(|| "full".to_owned()),
        models: input.models,
    })
}

pub(super) fn load_lm2_note_head_model(
    active_primary_model_sha256: Option<&str>,
) -> Result<Option<Lm2NoteHeadModel>, String> {
    let candidates = lm2_note_head_runtime_asset_candidates(LM2_NOTE_HEAD_MODEL_FILE);
    let Some(path) = candidates.iter().find(|path| path.is_file()).cloned() else {
        return Ok(None);
    };
    verify_model_asset_hash(&path, "note_head_model")?;
    let asset_sha256 = sha256_hex_of_file(&path)?;
    let input: Lm2NoteHeadModelFile = read_json_file(&path)?;
    match input.schema_version.as_str() {
        LM2_NOTE_HEAD_SCHEMA_V1 if input.feature_count == LM2_NOTE_HEAD_FEATURE_COUNT_V1 => {}
        LM2_NOTE_HEAD_SCHEMA_V2 | LM2_NOTE_HEAD_SCHEMA_V3 => {
            let expected_feature_count = if input.schema_version == LM2_NOTE_HEAD_SCHEMA_V3 {
                LM2_NOTE_HEAD_FEATURE_COUNT_V3
            } else {
                LM2_NOTE_HEAD_FEATURE_COUNT_V2
            };
            if input.feature_count != expected_feature_count
                || input.numeric_feature_count != Some(LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len())
                || input.primary_probability_order != ["hide_noise", "keep", "marginalia"]
                || (input.schema_version == LM2_NOTE_HEAD_SCHEMA_V3
                    && input.candidate_neighbor_offsets != LM2_NOTE_HEAD_NEIGHBOR_OFFSETS_V3)
            {
                return Err(format!(
                    "LM2 stacked note-head feature contract mismatch: f{}/numeric{:?}/probability_order={:?}/offsets={:?}",
                    input.feature_count,
                    input.numeric_feature_count,
                    input.primary_probability_order,
                    input.candidate_neighbor_offsets,
                ));
            }
            let expected_primary = input.primary_model_sha256.as_deref().ok_or_else(|| {
                "LM2 stacked note-head model is missing primary_model_sha256".to_owned()
            })?;
            let active_primary = active_primary_model_sha256.ok_or_else(|| {
                "LM2 stacked note-head model requires its exact CatBoost primary".to_owned()
            })?;
            if !expected_primary.eq_ignore_ascii_case(active_primary) {
                return Err(format!(
                    "LM2 stacked note-head primary-model mismatch: expected {expected_primary}, active {active_primary}"
                ));
            }
        }
        _ => {
            return Err(format!(
                "LM2 note-head model contract mismatch: schema={} features={}",
                input.schema_version, input.feature_count
            ));
        }
    }
    if input.baseline_prediction.len() != 1
        || input.trees.is_empty()
        || !input.threshold.is_finite()
        || !(0.0..=1.0).contains(&input.threshold)
        || input.trees.iter().any(|tree| tree.is_empty())
    {
        return Err("LM2 note-head model has an invalid binary HGB structure".to_owned());
    }
    Ok(Some(Lm2NoteHeadModel {
        schema_version: input.schema_version,
        feature_count: input.feature_count,
        primary_model_sha256: input.primary_model_sha256,
        baseline_prediction: input.baseline_prediction[0],
        trees: input.trees,
        threshold: input.threshold,
        asset_sha256,
    }))
}

pub(super) fn load_lm2_link_ranker_model(
    auth_model: Option<&Lm2NoteHeadModel>,
) -> Result<Option<Lm2LinkRankerModel>, String> {
    let candidates = lm2_link_ranker_runtime_asset_candidates(LM2_LINK_RANKER_MODEL_FILE);
    let Some(path) = candidates.iter().find(|path| path.is_file()).cloned() else {
        return Ok(None);
    };
    verify_model_asset_hash(&path, "link_ranker_model")?;
    let asset_sha256 = sha256_hex_of_file(&path)?;
    let input: Lm2LinkRankerModelFile = read_json_file(&path)?;
    let expected_names = LM2_LINK_RANKER_FEATURES_V1.map(str::to_owned);
    if input.schema_version != LM2_LINK_RANKER_SCHEMA_V1
        || input.feature_count != LM2_LINK_RANKER_FEATURE_COUNT_V1
        || input.feature_names != expected_names
        || input.candidate_page_offsets != [0, 1]
    {
        return Err(format!(
            "LM2 link-ranker feature contract mismatch: schema={} f{} offsets={:?}",
            input.schema_version, input.feature_count, input.candidate_page_offsets
        ));
    }
    let auth_model = auth_model.ok_or_else(|| {
        "LM2 link ranker requires its exact learned note authenticator".to_owned()
    })?;
    if auth_model.schema_version != LM2_NOTE_HEAD_SCHEMA_V3
        || !input
            .auth_model_sha256
            .eq_ignore_ascii_case(&auth_model.asset_sha256)
    {
        return Err(format!(
            "LM2 link-ranker authenticator mismatch: expected {}, active {}",
            input.auth_model_sha256, auth_model.asset_sha256
        ));
    }
    if input.baseline_prediction.len() != 1
        || input.trees.is_empty()
        || input.trees.iter().any(|tree| tree.is_empty())
        || !input.threshold.is_finite()
        || !(0.0..=1.0).contains(&input.threshold)
        || !input.auth_threshold.is_finite()
        || !(0.0..=1.0).contains(&input.auth_threshold)
    {
        return Err("LM2 link ranker has an invalid binary HGB policy".to_owned());
    }
    Ok(Some(Lm2LinkRankerModel {
        baseline_prediction: input.baseline_prediction[0],
        trees: input.trees,
        threshold: input.threshold,
        auth_threshold: input.auth_threshold,
        auth_model_sha256: input.auth_model_sha256,
        asset_sha256,
    }))
}

pub(super) fn lm2_context_twopass_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_CONTEXT_TWOPASS_MODEL").map(PathBuf::from) {
        candidates.push(path);
    }
    candidates.extend(lm2_runtime_asset_candidates(
        LM2_CONTEXT_TWOPASS_RUNTIME_DIR,
        file_name,
    ));
    candidates
}

pub(super) fn lm2_context_arbiter_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_CONTEXT_ARBITER_MODEL").map(PathBuf::from) {
        candidates.push(path);
    }
    candidates.extend(lm2_runtime_asset_candidates(
        LM2_CONTEXT_TWOPASS_RUNTIME_DIR,
        file_name,
    ));
    candidates
}

pub(super) fn lm2_note_head_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_NOTE_HEAD_MODEL").map(PathBuf::from) {
        candidates.push(path);
    }
    candidates.extend(lm2_runtime_asset_candidates(
        LM2_NOTE_HEAD_RUNTIME_DIR,
        file_name,
    ));
    candidates
}

pub(super) fn lm2_link_ranker_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_LINK_RANKER_MODEL").map(PathBuf::from) {
        candidates.push(path);
    }
    candidates.extend(lm2_runtime_asset_candidates(
        LM2_LINK_RANKER_RUNTIME_DIR,
        file_name,
    ));
    candidates
}

pub(super) fn lm2_native_catboost_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    lm2_runtime_asset_candidates(LM2_NATIVE_CATBOOST_RUNTIME_DIR, file_name)
}

pub(super) fn lm2_fasttab_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    lm2_runtime_asset_candidates(LM2_FASTTAB_RUNTIME_DIR, file_name)
}

pub(super) fn lm2_runtime_asset_candidates(runtime_dir: &str, file_name: &str) -> Vec<PathBuf> {
    let model_dir = std::env::var_os("LAWPDF_MODEL_DIR").map(PathBuf::from);
    let exe_path = std::env::current_exe().ok();
    lm2_runtime_asset_candidates_for(
        model_dir.as_deref(),
        exe_path.as_deref(),
        runtime_dir,
        file_name,
    )
}

pub(super) fn lm2_runtime_asset_candidates_for(
    model_dir: Option<&Path>,
    exe_path: Option<&Path>,
    runtime_dir: &str,
    file_name: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(model_dir) = model_dir {
        let relative_runtime_dir = runtime_dir
            .strip_prefix("profile-models/")
            .unwrap_or(runtime_dir);
        candidates.push(model_dir.join(relative_runtime_dir).join(file_name));
    }
    if let Some(exe_dir) = exe_path.and_then(Path::parent) {
        candidates.push(exe_dir.join(runtime_dir).join(file_name));
        candidates.push(
            exe_dir
                .join("..")
                .join("Resources")
                .join(runtime_dir)
                .join(file_name),
        );
    }
    candidates
}

pub(super) fn missing_model_error(label: &str, candidates: &[PathBuf]) -> String {
    let probed = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    format!("Missing {label}; probed: {probed}")
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ReleaseManifest {
    pub(super) runtime_assets: Lm2ReleaseRuntimeAssets,
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ReleaseRuntimeAssets {
    pub(super) fasttab_model: Lm2ReleaseAsset,
    pub(super) native_model: Lm2ReleaseAsset,
    pub(super) context_model: Lm2ReleaseAsset,
    #[serde(default)]
    pub(super) context_arbiter_model: Option<Lm2ReleaseAsset>,
    #[serde(default)]
    pub(super) note_head_model: Option<Lm2ReleaseAsset>,
    #[serde(default)]
    pub(super) link_ranker_model: Option<Lm2ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ReleaseAsset {
    pub(super) sha256: String,
}

pub(super) fn verify_model_asset_hash(path: &Path, asset_name: &str) -> Result<(), String> {
    let Some(manifest_path) = path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .take(3)
        .map(|directory| directory.join("release-manifest.json"))
        .find(|candidate| candidate.is_file())
    else {
        return Ok(());
    };
    let bytes = std::fs::read(&manifest_path).map_err(|error| {
        format!(
            "Could not read LM2 release manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: Lm2ReleaseManifest = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Could not decode LM2 release manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let expected = match asset_name {
        "fasttab_model" => Some(&manifest.runtime_assets.fasttab_model.sha256),
        "native_model" => Some(&manifest.runtime_assets.native_model.sha256),
        "context_model" => Some(&manifest.runtime_assets.context_model.sha256),
        "context_arbiter_model" => manifest
            .runtime_assets
            .context_arbiter_model
            .as_ref()
            .map(|asset| &asset.sha256),
        "note_head_model" => manifest
            .runtime_assets
            .note_head_model
            .as_ref()
            .map(|asset| &asset.sha256),
        "link_ranker_model" => manifest
            .runtime_assets
            .link_ranker_model
            .as_ref()
            .map(|asset| &asset.sha256),
        _ => return Err(format!("Unknown LM2 release-manifest asset: {asset_name}")),
    }
    .ok_or_else(|| format!("LM2 release manifest is missing {asset_name}"))?;
    let actual = sha256_hex_of_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "LM2 {asset_name} checksum mismatch for {}; expected {expected}, found {actual}",
            path.display()
        ));
    }
    Ok(())
}
