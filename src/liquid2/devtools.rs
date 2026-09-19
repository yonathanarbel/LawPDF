//! Release QA and runtime score import. Model-development CLIs are private.

use super::*;

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Deserialize)]
pub(super) struct Lm2EvalExamplesFile {
    pub(super) lines: Vec<Lm2EvalRow>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Deserialize)]
pub(super) struct Lm2EvalLabelsFile {
    pub(super) labels: Vec<Lm2EvalLabel>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Clone, Deserialize)]
pub(super) struct Lm2EvalRow {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) line_index: usize,
    pub(super) text: String,
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) page_width: Option<f32>,
    #[serde(default)]
    pub(super) page_height: Option<f32>,
    #[serde(default)]
    pub(super) x0: Option<f32>,
    #[serde(default)]
    pub(super) y0: Option<f32>,
    #[serde(default)]
    pub(super) x1: Option<f32>,
    #[serde(default)]
    pub(super) y1: Option<f32>,
    #[serde(default)]
    pub(super) font_size: Option<f32>,
    #[serde(default)]
    pub(super) font_ratio_page: Option<f32>,
    #[serde(default)]
    pub(super) font_ratio_page_ref: Option<f32>,
    #[serde(default)]
    pub(super) font_ratio_doc: Option<f32>,
    #[serde(default)]
    pub(super) bold: Option<bool>,
    #[serde(default)]
    pub(super) italic: Option<bool>,
    #[serde(default)]
    pub(super) centered: Option<bool>,
    #[serde(default)]
    pub(super) below_footnote_divider: Option<bool>,
    #[serde(default)]
    pub(super) page_has_footnote_divider: Option<bool>,
    #[serde(default)]
    pub(super) in_footnote_zone: Option<bool>,
    #[serde(default)]
    pub(super) page_object_image_overlap_ratio: Option<f32>,
    #[serde(default)]
    pub(super) page_object_image_hit_count: Option<u16>,
    #[serde(default)]
    pub(super) page_object_path_stroke_near_line_count: Option<u16>,
    #[serde(default)]
    pub(super) page_object_path_stroke_density_near_line: Option<f32>,
    #[serde(default)]
    pub(super) page_object_thin_horizontal_near_line_count: Option<u16>,
    #[serde(default)]
    pub(super) page_object_thin_vertical_near_line_count: Option<u16>,
    #[serde(default)]
    pub(super) page_object_overlaps_image_bbox: Option<bool>,
    #[serde(default)]
    pub(super) page_object_ruled_row_membership: Option<bool>,
    #[serde(default)]
    pub(super) page_object_hide_candidate: Option<bool>,
    #[serde(default)]
    pub(super) page_object_hide_candidate_guarded: Option<bool>,
    #[serde(default)]
    pub(super) page_object_path15_candidate: Option<bool>,
    #[serde(default)]
    pub(super) page_object_ruled_or_path8_candidate: Option<bool>,
    #[serde(default)]
    pub(super) line_on_ruled_divider: Option<bool>,
    #[serde(default)]
    pub(super) in_ruled_cell: Option<bool>,
    #[serde(default)]
    pub(super) ruled_row_membership_exact: Option<bool>,
    #[serde(default)]
    pub(super) dist_to_nearest_rule: Option<f32>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Deserialize)]
pub(super) struct Lm2EvalLabel {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) role: String,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2EvalReport {
    pub(super) model_label: String,
    pub(super) pp_prior_source: Option<String>,
    pub(super) pp_footnote_region_membership: bool,
    pub(super) external_emissions_input: Option<String>,
    pub(super) examples_input: String,
    pub(super) labels_input: String,
    pub(super) total: usize,
    pub(super) accuracy: f64,
    pub(super) macro_f1: f64,
    pub(super) per_action: Vec<Lm2EvalActionMetric>,
    pub(super) confusion: [[usize; 3]; 3],
    pub(super) matched_rows: usize,
    pub(super) label_rows: usize,
    pub(super) use_example_role_hints: bool,
    pub(super) block_quality: Lm2BlockQualityMetrics,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2EvalActionMetric {
    pub(super) action: &'static str,
    pub(super) support: usize,
    pub(super) precision: f64,
    pub(super) recall: f64,
    pub(super) f1: f64,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug)]
pub(super) struct Lm2EvalItem {
    pub(super) source: DeepLiquidSourceLine,
    pub(super) actual: Lm2Action,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2EvalDisagreement {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) line_index: usize,
    pub(super) actual: &'static str,
    pub(super) predicted: &'static str,
    pub(super) previous_text: Option<String>,
    pub(super) text: String,
    pub(super) next_text: Option<String>,
    pub(super) y_bottom: f32,
    pub(super) font_ratio_page: f32,
    pub(super) font_ratio_doc: f32,
    pub(super) doc_footnote_state: bool,
    pub(super) doc_footnote_continuation: bool,
    pub(super) doc_note_marker: u16,
    pub(super) doc_note_marker_first_on_page: bool,
    pub(super) doc_note_marker_mid_sequence_page: bool,
    pub(super) doc_note_marker_follows_previous_page: bool,
    pub(super) doc_note_marker_page_delta: i16,
    pub(super) below_footnote_divider: bool,
    pub(super) page_has_footnote_divider: bool,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2BlockQualityMetrics {
    pub(super) block_count: usize,
    pub(super) marginalia_blocks: usize,
    pub(super) marginalia_source_lines: usize,
    pub(super) mean_lines_per_marginalia_block: f64,
    pub(super) paragraph_blocks: usize,
    pub(super) distinct_pages: usize,
    pub(super) paragraphs_per_page: f64,
    pub(super) hyphen_artifacts: usize,
    pub(super) hyphen_artifacts_per_1000_blocks: f64,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Default)]
pub(super) struct Lm2BlockQualityAccumulator {
    pub(super) block_count: usize,
    pub(super) marginalia_blocks: usize,
    pub(super) marginalia_source_lines: usize,
    pub(super) paragraph_blocks: usize,
    pub(super) evaluated_pages: usize,
    pub(super) hyphen_artifacts: usize,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2FeatureDumpReport {
    pub(super) examples_input: String,
    pub(super) feature_dim: usize,
    pub(super) doc_font_zscores: bool,
    pub(super) repetition_fingerprints: bool,
    pub(super) marker_continuity: bool,
    pub(super) rows: Vec<Lm2FeatureDumpRow>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2FeatureDumpRow {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) features: Vec<(usize, f64)>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2DecoderLatticeReport {
    pub(super) schema_version: &'static str,
    pub(super) model_label: String,
    pub(super) examples_input: String,
    pub(super) labels_input: String,
    pub(super) matched_rows: usize,
    pub(super) label_rows: usize,
    pub(super) page_count: usize,
    pub(super) line_count: usize,
    pub(super) use_example_role_hints: bool,
    pub(super) pages: Vec<Lm2DecoderLatticePage>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2DecoderLatticePage {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) lines: Vec<Lm2DecoderLatticeLine>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2DecoderLatticeLine {
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) gold_action: &'static str,
    pub(super) role_hint: Option<&'static str>,
    pub(super) emission_scores_after_priors: BTreeMap<String, f64>,
    pub(super) start_features: BTreeMap<String, f64>,
    pub(super) start_scores: BTreeMap<String, f64>,
    pub(super) transition_features_from_previous: Option<BTreeMap<String, f64>>,
    pub(super) transition_scores_from_previous: Option<BTreeMap<String, f64>>,
    pub(super) y_bottom_ratio: f32,
    pub(super) font_ratio_page: f32,
    pub(super) font_ratio_doc: f32,
    pub(super) doc_footnote_state: bool,
    pub(super) doc_footnote_continuation: bool,
    pub(super) doc_note_marker: u16,
    pub(super) doc_note_marker_first_on_page: bool,
    pub(super) doc_note_marker_mid_sequence_page: bool,
    pub(super) doc_note_marker_follows_previous_page: bool,
    pub(super) doc_note_marker_page_delta: i16,
    pub(super) below_footnote_divider: bool,
    pub(super) page_has_footnote_divider: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ExternalEmissionsFile {
    #[serde(default)]
    pub(super) schema_version: Option<String>,
    #[serde(default)]
    pub(super) model_label: Option<String>,
    pub(super) pages: Vec<Lm2ExternalEmissionsPage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ExternalEmissionsPage {
    pub(super) path: String,
    pub(super) page_index: usize,
    pub(super) lines: Vec<Lm2ExternalEmissionsLine>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Lm2ExternalEmissionsLine {
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) emission_scores_after_priors: BTreeMap<String, f64>,
}

#[derive(Debug)]
pub(super) struct Lm2ExternalEmissions {
    pub(super) source_path: PathBuf,
    pub(super) schema_version: Option<String>,
    pub(super) model_label: Option<String>,
    pub(super) scores_by_key: HashMap<String, [f64; 3]>,
    pub(super) basename_scores_by_key: HashMap<String, [f64; 3]>,
}

impl Lm2ExternalEmissions {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let input: Lm2ExternalEmissionsFile = read_json_file(path)?;
        let mut scores_by_key = HashMap::new();
        let mut basename_scores_by_key = HashMap::new();
        let mut ambiguous_basename_keys = HashSet::new();
        let mut rows = 0usize;
        for page in input.pages {
            for line in page.lines {
                let scores = parse_external_action_scores(
                    &line.emission_scores_after_priors,
                    &page.path,
                    page.page_index,
                    line.line_index,
                )?;
                let path_key = external_full_path_key(&page.path);
                let key = eval_key(&path_key, page.page_index, line.line_index, &line.text);
                scores_by_key.entry(key).or_insert(scores);
                if let Some(basename_key) = external_basename_key(&page.path) {
                    let key = eval_key(&basename_key, page.page_index, line.line_index, &line.text);
                    if let Some(previous) = basename_scores_by_key.insert(key.clone(), scores)
                        && previous != scores
                    {
                        ambiguous_basename_keys.insert(key);
                    }
                }
                rows += 1;
            }
        }
        for key in ambiguous_basename_keys {
            basename_scores_by_key.remove(&key);
        }
        if rows == 0 {
            return Err(format!(
                "external emissions file has no rows: {}",
                path.display()
            ));
        }
        Ok(Self {
            source_path: path.to_path_buf(),
            schema_version: input.schema_version,
            model_label: input.model_label,
            scores_by_key,
            basename_scores_by_key,
        })
    }

    pub(super) fn page_scores(
        &self,
        document_path: &Path,
        lines: &[DeepLiquidSourceLine],
    ) -> Result<Vec<[f64; 3]>, String> {
        let path_keys = external_document_path_keys(document_path);
        lines
            .iter()
            .map(|line| {
                for path_key in &path_keys {
                    let key = eval_key(path_key, line.page_index, line.line_index, &line.text);
                    if let Some(scores) = self.scores_by_key.get(&key) {
                        return Ok(*scores);
                    }
                }
                if let Some(basename_key) = document_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(external_full_path_key)
                {
                    let key = eval_key(&basename_key, line.page_index, line.line_index, &line.text);
                    if let Some(scores) = self.basename_scores_by_key.get(&key) {
                        return Ok(*scores);
                    }
                }
                Err(format!(
                    "missing external emissions for {} page {} line {} text {:?} in {}",
                    document_path.display(),
                    line.page_index,
                    line.line_index,
                    collapse_whitespace(&line.text),
                    self.source_path.display()
                ))
            })
            .collect()
    }

    pub(super) fn source_label(&self) -> String {
        let model = self.model_label.as_deref().unwrap_or("unknown-model");
        let schema = self.schema_version.as_deref().unwrap_or("unknown-schema");
        format!("{} ({schema}, {model})", self.source_path.display())
    }
}

pub(super) fn parse_external_action_scores(
    scores: &BTreeMap<String, f64>,
    path: &str,
    page_index: usize,
    line_index: usize,
) -> Result<[f64; 3], String> {
    let mut parsed = [0.0; 3];
    for action in ACTIONS {
        let Some(value) = scores.get(action.as_str()) else {
            return Err(format!(
                "external emissions row missing {} score for {} page {} line {}",
                action.as_str(),
                path,
                page_index,
                line_index
            ));
        };
        if !value.is_finite() {
            return Err(format!(
                "external emissions row has non-finite {} score for {} page {} line {}",
                action.as_str(),
                path,
                page_index,
                line_index
            ));
        }
        parsed[action.index()] = *value;
    }
    Ok(parsed)
}

pub(super) fn external_document_path_keys(path: &Path) -> Vec<String> {
    let mut keys = vec![external_full_path_key(&path.display().to_string())];
    if let Ok(canonical) = std::fs::canonicalize(path) {
        keys.push(external_full_path_key(&canonical.display().to_string()));
    }
    keys.sort();
    keys.dedup();
    keys
}

pub(super) fn external_full_path_key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

pub(super) fn external_basename_key(path: &str) -> Option<String> {
    let normalized = external_full_path_key(path);
    normalized
        .rsplit('/')
        .next()
        .filter(|file_name| !file_name.is_empty())
        .map(str::to_owned)
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Deserialize)]
pub(super) struct Lm2DraftInput {
    pub(super) documents: Vec<Lm2DraftInputDocument>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Deserialize)]
pub(super) struct Lm2DraftInputDocument {
    pub(super) path: String,
    #[serde(default)]
    pub(super) selection_manifest_index: Option<usize>,
    #[serde(default)]
    pub(super) selection_primary_stratum: Option<String>,
    #[serde(default)]
    pub(super) selection_stratum_tags: Vec<String>,
    #[serde(default)]
    pub(super) source_lines: Vec<DeepLiquidSourceLine>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2SourceSmokeReport {
    pub(super) app_version: &'static str,
    pub(super) timestamp_unix_secs: u64,
    pub(super) document_count: usize,
    pub(super) failures: usize,
    pub(super) documents: Vec<Lm2SourceSmokeDocument>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2SourceSmokeDocument {
    pub(super) path: String,
    pub(super) source_lines: Vec<DeepLiquidSourceLine>,
    pub(super) block_source_lines: Vec<LiquidBlockSourceLines>,
    pub(super) error: Option<String>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2DraftReport {
    pub(super) schema_version: &'static str,
    pub(super) model_label: String,
    pub(super) applied_runtime_context: bool,
    pub(super) runtime_context_layers: Vec<String>,
    pub(super) pp_prior_source: Option<String>,
    pub(super) pp_footnote_region_membership: bool,
    pub(super) source_input: String,
    pub(super) document_count: usize,
    pub(super) row_count: usize,
    pub(super) rows: Vec<Lm2DraftRow>,
}

#[cfg(any(feature = "devtools", test))]
#[derive(Debug, Serialize)]
pub(super) struct Lm2DraftRow {
    pub(super) path: String,
    pub(super) selection_manifest_index: Option<usize>,
    pub(super) selection_primary_stratum: Option<String>,
    pub(super) selection_stratum_tags: Vec<String>,
    pub(super) line_id: String,
    pub(super) page_index: usize,
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) lm2_action: &'static str,
    pub(super) lm1_hint_action: &'static str,
    pub(super) role_hint: Option<&'static str>,
    pub(super) y_bottom_ratio: f32,
    pub(super) font_ratio_page: f32,
    pub(super) font_ratio_doc: f32,
    pub(super) doc_footnote_state: bool,
    pub(super) doc_footnote_continuation: bool,
    pub(super) doc_note_marker: u16,
    pub(super) doc_note_marker_first_on_page: bool,
    pub(super) doc_note_marker_mid_sequence_page: bool,
    pub(super) doc_note_marker_follows_previous_page: bool,
    pub(super) doc_note_marker_page_delta: i16,
    pub(super) below_footnote_divider: bool,
    pub(super) page_has_footnote_divider: bool,
}

#[cfg(feature = "devtools")]
pub fn run_lm2_eval(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut examples_input = PathBuf::new();
    let mut labels_input = PathBuf::new();
    let mut output_path: Option<PathBuf> = None;
    let mut disagreements_output_path: Option<PathBuf> = None;
    let mut disagreements_limit = 100usize;
    let mut use_example_role_hints = false;
    let mut external_emissions_path: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-eval") {
            continue;
        }
        if arg == OsStr::new("--examples-input") {
            examples_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--examples-input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--labels") {
            labels_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--labels needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--disagreements-output") {
            disagreements_output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--disagreements-output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--disagreements-limit") {
            let value = args
                .next()
                .ok_or_else(|| "--disagreements-limit needs a value".to_owned())?;
            disagreements_limit = value
                .to_string_lossy()
                .parse::<usize>()
                .map_err(|_| "--disagreements-limit must be a non-negative integer".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--lm2-external-emissions")
            || arg == OsStr::new("--external-emissions")
        {
            external_emissions_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--lm2-external-emissions needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--use-example-role-hints") {
            use_example_role_hints = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 eval argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    let runtime = Lm2Runtime::load(Lm2RuntimeChoice::Automatic);
    let external_emissions = external_emissions_path
        .as_deref()
        .map(Lm2ExternalEmissions::load)
        .transpose()?;
    if examples_input.as_os_str().is_empty() || labels_input.as_os_str().is_empty() {
        return Err("Release QA requires explicit --examples and --labels paths to your private fidelity corpus.".to_owned());
    }
    let examples: Lm2EvalExamplesFile = read_json_file(&examples_input)?;
    let labels: Lm2EvalLabelsFile = read_json_file(&labels_input)?;
    let mut label_by_key = HashMap::new();
    for label in &labels.labels {
        label_by_key.insert(
            eval_key(&label.path, label.page_index, label.line_index, &label.text),
            label,
        );
    }

    let mut groups: BTreeMap<(String, usize), Vec<Lm2EvalItem>> = BTreeMap::new();
    let mut matched_rows = 0usize;
    for (row, mut source) in eval_sources_for_rows(examples.lines, use_example_role_hints) {
        let Some(label) = label_by_key.get(&eval_key(
            &row.path,
            row.page_index,
            row.line_index,
            &row.text,
        )) else {
            continue;
        };
        annotate_pp_prior(&runtime, &row.path, &mut source);
        matched_rows += 1;
        groups
            .entry((row.path.to_ascii_lowercase(), row.page_index))
            .or_default()
            .push(Lm2EvalItem {
                source,
                actual: action_for_role_name(&label.role),
            });
    }

    let mut confusion = [[0usize; 3]; 3];
    let mut block_quality = Lm2BlockQualityAccumulator::default();
    let mut disagreements = Vec::new();
    for ((path, page_index), mut items) in groups {
        items.sort_by_key(|item| item.source.line_index);
        let sources = items
            .iter()
            .map(|item| item.source.clone())
            .collect::<Vec<_>>();
        let external_scores = external_emissions
            .as_ref()
            .map(|external| external.page_scores(Path::new(&path), &sources))
            .transpose()?;
        let decoded = decode_page_with_emissions(&runtime, &sources, external_scores.as_deref());
        for (index, (item, (_, predicted))) in items.iter().zip(decoded.iter()).enumerate() {
            confusion[item.actual.index()][predicted.index()] += 1;
            if item.actual != *predicted {
                disagreements.push(Lm2EvalDisagreement {
                    path: path.clone(),
                    page_index,
                    line_index: item.source.line_index,
                    actual: item.actual.as_str(),
                    predicted: predicted.as_str(),
                    previous_text: index
                        .checked_sub(1)
                        .and_then(|previous| items.get(previous))
                        .map(|previous| previous.source.text.clone()),
                    text: item.source.text.clone(),
                    next_text: items.get(index + 1).map(|next| next.source.text.clone()),
                    y_bottom: item.source.bottom / item.source.page_height.max(1.0),
                    font_ratio_page: item.source.font_ratio_page,
                    font_ratio_doc: item.source.font_ratio_doc,
                    doc_footnote_state: item.source.doc_footnote_state,
                    doc_footnote_continuation: item.source.doc_footnote_continuation,
                    doc_note_marker: item.source.doc_note_marker,
                    doc_note_marker_first_on_page: item.source.doc_note_marker_first_on_page,
                    doc_note_marker_mid_sequence_page: item
                        .source
                        .doc_note_marker_mid_sequence_page,
                    doc_note_marker_follows_previous_page: item
                        .source
                        .doc_note_marker_follows_previous_page,
                    doc_note_marker_page_delta: item.source.doc_note_marker_page_delta,
                    below_footnote_divider: item.source.below_footnote_divider,
                    page_has_footnote_divider: item.source.page_has_footnote_divider,
                });
            }
        }
        block_quality.add_page(&decoded);
    }
    if let Some(path) = disagreements_output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut jsonl = String::new();
        for row in disagreements.iter().take(disagreements_limit) {
            jsonl.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
            jsonl.push('\n');
        }
        std::fs::write(&path, jsonl).map_err(|error| error.to_string())?;
    }
    let pp_prior_source = runtime.pp_prior_source();
    let report = lm2_eval_report(
        runtime.model_label,
        pp_prior_source,
        runtime.pp_footnote_region_membership,
        external_emissions_path.as_deref(),
        &examples_input,
        &labels_input,
        confusion,
        matched_rows,
        labels.labels.len(),
        use_example_role_hints,
        block_quality.finish(),
    );
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

#[cfg(feature = "devtools")]
pub fn run_lm2_source_smoke(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut input_path = PathBuf::new();
    let mut output_path: Option<PathBuf> = None;
    let mut external_emissions_path: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-source-smoke") {
            continue;
        }
        if arg == OsStr::new("--input") {
            input_path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--lm2-external-emissions")
            || arg == OsStr::new("--external-emissions")
        {
            external_emissions_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--lm2-external-emissions needs a path".to_owned())?,
            );
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 source smoke argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    if input_path.as_os_str().is_empty() {
        return Err("Source QA requires an explicit --input path.".to_owned());
    }
    let input: Lm2DraftInput = read_json_file(&input_path)?;
    let mut documents = Vec::with_capacity(input.documents.len());
    for document in input.documents {
        let source_lines = document
            .source_lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let request = LiquidMode2Request {
            document_epoch: 0,
            path: PathBuf::from(&document.path),
            title: document.path.clone(),
            pages: Vec::new(),
            deep_source_lines: source_lines.clone(),
            use_pymupdf_blocks: false,
            use_pp_footnote_regions: false,
            external_emissions_path: external_emissions_path.clone(),
            runtime_choice: Lm2RuntimeChoice::Automatic,
            preview_only: false,
            skip_progressive_preview: false,
        };
        match prepare_liquid_mode2_document(request) {
            Ok(liquid) => documents.push(Lm2SourceSmokeDocument {
                path: document.path,
                source_lines,
                block_source_lines: liquid.block_source_lines,
                error: None,
            }),
            Err(error) => documents.push(Lm2SourceSmokeDocument {
                path: document.path,
                source_lines,
                block_source_lines: Vec::new(),
                error: Some(error),
            }),
        }
    }
    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();
    let report = Lm2SourceSmokeReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        document_count: documents.len(),
        failures,
        documents,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}
