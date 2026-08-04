use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use blake2b_simd::Params;
use tract_onnx::prelude::*;

use super::*;

const FASTTAB_CATEGORY_BUCKETS: u64 = 4096;
const FASTTAB_TEXT_BYTES: usize = 256;
const FASTTAB_TEXT_HEAD_BYTES: usize = 192;
const FASTTAB_BATCH_SIZE: usize = 128;

type FastTabPlan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub(super) struct Lm2FastTabModel {
    model: FastTabPlan,
    path: PathBuf,
}

impl fmt::Debug for Lm2FastTabModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Lm2FastTabModel")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Lm2FastTabModel {
    pub(super) fn load() -> Result<Option<Self>, String> {
        if !fasttab_enabled() {
            return Ok(None);
        }
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("LAWPDF_LM2_FASTTAB_MODEL").map(PathBuf::from) {
            candidates.push(path);
        }
        candidates.extend(lm2_fasttab_runtime_asset_candidates(LM2_FASTTAB_MODEL_FILE));
        let Some(path) = candidates.iter().find(|path| path.is_file()).cloned() else {
            return Ok(None);
        };
        Self::load_path(path).map(Some)
    }

    fn load_path(path: PathBuf) -> Result<Self, String> {
        verify_model_asset_hash(&path, "fasttab_model")?;
        let model = tract_onnx::onnx()
            .model_for_path(&path)
            .map_err(|error| format!("Could not read FastTab ONNX model: {error}"))?
            .into_optimized()
            .map_err(|error| format!("Could not optimize FastTab ONNX model: {error}"))?
            .into_runnable()
            .map_err(|error| format!("Could not prepare FastTab ONNX model: {error}"))?;
        let runtime = Self { model, path };
        let probe = runtime.run_encoded(&vec![0.0; 116], &vec![0; 14], &vec![0; 256], 1)?;
        if probe.len() != 1 || !probe[0].iter().all(|value| value.is_finite()) {
            return Err("FastTab ONNX load probe returned invalid scores".to_owned());
        }
        Ok(runtime)
    }

    pub(super) fn emission_scores(
        &self,
        lines: &[DeepLiquidSourceLine],
    ) -> Result<Vec<[f64; 3]>, String> {
        let mut output = Vec::with_capacity(lines.len());
        let mut category_caches = (0..LM2_NATIVE_CATBOOST_CAT_FEATURES.len())
            .map(|_| HashMap::<String, i64>::new())
            .collect::<Vec<_>>();
        for chunk in lines.chunks(FASTTAB_BATCH_SIZE) {
            let mut numeric =
                Vec::with_capacity(chunk.len() * LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len());
            let mut categories =
                Vec::with_capacity(chunk.len() * LM2_NATIVE_CATBOOST_CAT_FEATURES.len());
            let mut text = Vec::with_capacity(chunk.len() * FASTTAB_TEXT_BYTES);
            for line in chunk {
                let feature_map = lm2_numeric_catboost_features(line);
                numeric.extend(
                    LM2_NATIVE_CATBOOST_FLOAT_FEATURES
                        .iter()
                        .map(|name| finite_f32(feature_map.get(*name).copied().unwrap_or(0.0))),
                );
                let category_values = lm2_native_catboost_cat_features(line);
                for (column, value) in category_values.into_iter().enumerate() {
                    let encoded = if let Some(encoded) = category_caches[column].get(&value) {
                        *encoded
                    } else {
                        let encoded = stable_category_hash(column, &value);
                        category_caches[column].insert(value, encoded);
                        encoded
                    };
                    categories.push(encoded);
                }
                let collapsed = collapse_whitespace(&line.text)
                    .chars()
                    .take(500)
                    .collect::<String>();
                text.extend(encode_text(&collapsed));
            }
            output.extend(self.run_encoded(&numeric, &categories, &text, chunk.len())?);
        }
        Ok(output)
    }

    fn run_encoded(
        &self,
        numeric: &[f32],
        categories: &[i64],
        text: &[i64],
        batch: usize,
    ) -> Result<Vec<[f64; 3]>, String> {
        let numeric =
            Tensor::from_shape(&[batch, LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len()], numeric)
                .map_err(|error| format!("Could not build FastTab numeric tensor: {error}"))?;
        let categories =
            Tensor::from_shape(&[batch, LM2_NATIVE_CATBOOST_CAT_FEATURES.len()], categories)
                .map_err(|error| format!("Could not build FastTab category tensor: {error}"))?;
        let text = Tensor::from_shape(&[batch, FASTTAB_TEXT_BYTES], text)
            .map_err(|error| format!("Could not build FastTab text tensor: {error}"))?;
        let outputs = self
            .model
            .run(tvec!(numeric.into(), categories.into(), text.into()))
            .map_err(|error| format!("FastTab ONNX inference failed: {error}"))?;
        let scores = outputs
            .first()
            .ok_or_else(|| "FastTab ONNX returned no outputs".to_owned())?
            .to_array_view::<f32>()
            .map_err(|error| format!("FastTab ONNX output was not f32: {error}"))?;
        if scores.shape() != [batch, 3] {
            return Err(format!(
                "FastTab ONNX returned shape {:?}, expected [{batch}, 3]",
                scores.shape()
            ));
        }
        Ok(scores
            .outer_iter()
            .map(|row| [row[0] as f64, row[1] as f64, row[2] as f64])
            .collect())
    }
}

pub(super) fn fasttab_enabled() -> bool {
    std::env::var("LAWPDF_LM2_FASTTAB")
        .map(|value| fasttab_value_enabled(&value))
        .unwrap_or(false)
}

fn fasttab_value_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

fn finite_f32(value: f64) -> f32 {
    let value = value as f32;
    if value.is_nan() {
        0.0
    } else if value == f32::INFINITY {
        1_000_000.0
    } else if value == f32::NEG_INFINITY {
        -1_000_000.0
    } else {
        value
    }
}

fn stable_category_hash(column: usize, value: &str) -> i64 {
    let payload = format!("{column}\0{value}");
    let hash = Params::new()
        .hash_length(8)
        .personal(b"lawpdf-c")
        .hash(payload.as_bytes());
    let mut digest = [0u8; 8];
    digest.copy_from_slice(hash.as_bytes());
    (u64::from_le_bytes(digest) % FASTTAB_CATEGORY_BUCKETS + 1) as i64
}

fn encode_text(value: &str) -> [i64; FASTTAB_TEXT_BYTES] {
    let raw = value.as_bytes();
    let mut output = [0i64; FASTTAB_TEXT_BYTES];
    if raw.len() <= FASTTAB_TEXT_BYTES {
        for (destination, source) in output.iter_mut().zip(raw.iter()) {
            *destination = i64::from(*source);
        }
        return output;
    }
    for (destination, source) in output[..FASTTAB_TEXT_HEAD_BYTES]
        .iter_mut()
        .zip(raw[..FASTTAB_TEXT_HEAD_BYTES].iter())
    {
        *destination = i64::from(*source);
    }
    let tail = &raw[raw.len() - (FASTTAB_TEXT_BYTES - FASTTAB_TEXT_HEAD_BYTES)..];
    for (destination, source) in output[FASTTAB_TEXT_HEAD_BYTES..]
        .iter_mut()
        .zip(tail.iter())
    {
        *destination = i64::from(*source);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_hash_matches_training_encoder() {
        assert_eq!(stable_category_hash(0, "bin0"), 4095);
        assert_eq!(stable_category_hash(3, "x"), 3435);
        assert_eq!(stable_category_hash(13, "other"), 2327);
        assert_eq!(stable_category_hash(7, ""), 2932);
    }

    #[test]
    fn text_encoder_keeps_head_and_tail_bytes() {
        let value = (0..300)
            .map(|index| char::from(b'a' + (index % 26) as u8))
            .collect::<String>();
        let encoded = encode_text(&value);
        let expected_head = value.as_bytes()[..192]
            .iter()
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        let expected_tail = value.as_bytes()[236..]
            .iter()
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        assert_eq!(&encoded[..192], expected_head.as_slice());
        assert_eq!(&encoded[192..], expected_tail.as_slice());
    }

    #[test]
    fn numeric_sanitizer_matches_training_encoder() {
        assert_eq!(finite_f32(f64::NAN), 0.0);
        assert_eq!(finite_f32(f64::INFINITY), 1_000_000.0);
        assert_eq!(finite_f32(f64::NEG_INFINITY), -1_000_000.0);
        assert_eq!(finite_f32(f64::MAX), 1_000_000.0);
        assert_eq!(finite_f32(f64::MIN), -1_000_000.0);
    }

    #[test]
    fn fasttab_is_opt_in() {
        assert!(fasttab_value_enabled("yes"));
        assert!(fasttab_value_enabled(" TRUE "));
        assert!(!fasttab_value_enabled("0"));
        assert!(!fasttab_value_enabled(""));
    }
}
