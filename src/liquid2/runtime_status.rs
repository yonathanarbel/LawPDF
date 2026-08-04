use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use serde::Serialize;

use super::*;
use crate::liquidvision::LiquidVision;

#[derive(Debug, Serialize)]
struct Lm2RuntimeStatus {
    schema_version: &'static str,
    app_version: &'static str,
    platform: &'static str,
    runtime_tier: &'static str,
    model_label: String,
    fasttab_requested: bool,
    fasttab_active: bool,
    native_catboost_active: bool,
    native_line_model_active: bool,
    context_twopass_active: bool,
    liquidvision_active: bool,
    legacy_model_available: bool,
    fasttab_model_path: Option<String>,
    native_model_path: Option<String>,
    native_library_path: Option<String>,
    context_model_path: Option<String>,
    require_native: bool,
    require_context: bool,
    requirements_met: bool,
    errors: Vec<String>,
}

pub fn run_lm2_runtime_status(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut require_native = false;
    let mut require_context = false;
    let mut output_path: Option<PathBuf> = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-runtime-status") {
            continue;
        }
        if arg == OsStr::new("--require-native") {
            require_native = true;
            continue;
        }
        if arg == OsStr::new("--require-context") {
            require_context = true;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a destination path".to_owned())?,
            );
            continue;
        }
        return Err(format!(
            "unknown LM2 runtime-status argument: {}",
            arg.to_string_lossy()
        ));
    }

    let status = lm2_runtime_status(require_native, require_context);
    let json = serde_json::to_string_pretty(&status).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, format!("{json}\n")).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    if status.requirements_met {
        Ok(())
    } else {
        Err(status.errors.join("; "))
    }
}

fn lm2_runtime_status(require_native: bool, require_context: bool) -> Lm2RuntimeStatus {
    let fasttab_requested = fasttab_enabled();
    let fasttab_model_path = std::env::var_os("LAWPDF_LM2_FASTTAB_MODEL")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_fasttab_runtime_asset_candidates(LM2_FASTTAB_MODEL_FILE)
                .into_iter()
                .find(|path| path.is_file())
        });
    let native_model_path = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_MODEL")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_native_catboost_runtime_asset_candidates(LM2_NATIVE_CATBOOST_MODEL_FILE)
                .into_iter()
                .find(|path| path.is_file())
        });
    let native_library_path = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_LIB")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_native_catboost_runtime_asset_candidates(lm2_native_catboost_library_file())
                .into_iter()
                .find(|path| path.is_file())
        });
    let context_model_path =
        lm2_context_twopass_runtime_asset_candidates(LM2_CONTEXT_TWOPASS_MODEL_FILE)
            .into_iter()
            .find(|path| path.is_file());

    let mut errors = Vec::new();
    let fasttab_model = match Lm2FastTabModel::load() {
        Ok(model) => model,
        Err(error) => {
            errors.push(format!("FastTab load failed: {error}"));
            None
        }
    };
    let fasttab_active = fasttab_model.is_some();
    if fasttab_requested && !fasttab_active && errors.is_empty() {
        errors.push("requested FastTab ONNX model was not loaded".to_owned());
    }
    let native_model = match load_lm2_native_catboost_model() {
        Ok(model) => model,
        Err(error) => {
            errors.push(format!("native CatBoost load failed: {error}"));
            None
        }
    };
    let native_catboost_active = native_model.is_some();
    if require_native && !native_catboost_active && errors.is_empty() {
        errors.push("required CatBoost default model/library assets were not found".to_owned());
    }
    let native_line_model_active = fasttab_active || native_catboost_active;

    let context_model = if native_line_model_active {
        match load_lm2_context_twopass_model() {
            Ok(model) => model,
            Err(error) => {
                errors.push(format!("context two-pass load failed: {error}"));
                None
            }
        }
    } else {
        None
    };
    let context_twopass_active = context_model.is_some();
    if require_context && !context_twopass_active {
        errors.push("required context two-pass model was not loaded".to_owned());
    }
    let liquidvision_active =
        native_line_model_active && liquidvision_enabled(true) && LiquidVision::global().is_some();
    if require_native && !liquidvision_active {
        errors.push("required LiquidVision feature model was not loaded".to_owned());
    }

    let legacy_model = load_lm2_model().ok().filter(model_is_usable);
    let legacy_model_available = legacy_model.is_some();
    let (runtime_tier, model_label) = if fasttab_active {
        (
            if context_twopass_active {
                "fasttab_onnx_context"
            } else {
                "fasttab_onnx"
            },
            "lm2-fasttab-onnx:f116c14b256d3".to_owned(),
        )
    } else if let Some(model) = native_model.as_ref() {
        (
            if context_twopass_active {
                "native_catboost_context"
            } else {
                "native_catboost"
            },
            format!(
                "lm2-native-catboost-text-runtime:f{}c{}t{}d{}",
                model.float_feature_count,
                model.cat_feature_count,
                model.text_feature_count,
                model.dimensions_count
            ),
        )
    } else if let Some(model) = legacy_model.as_ref() {
        ("legacy_hashed_softmax", model.model_id.clone())
    } else {
        ("heuristic_fallback", "lm2-heuristic-fallback".to_owned())
    };
    let requirements_met = (!require_native || native_catboost_active)
        && (!require_native || liquidvision_active)
        && (!require_context || context_twopass_active)
        && errors.is_empty();

    Lm2RuntimeStatus {
        schema_version: "lm2-runtime-status-v2",
        app_version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        runtime_tier,
        model_label,
        fasttab_requested,
        fasttab_active,
        native_catboost_active,
        native_line_model_active,
        context_twopass_active,
        liquidvision_active,
        legacy_model_available,
        fasttab_model_path: fasttab_model_path.map(|path| path.display().to_string()),
        native_model_path: native_model_path.map(|path| path.display().to_string()),
        native_library_path: native_library_path.map(|path| path.display().to_string()),
        context_model_path: context_model_path.map(|path| path.display().to_string()),
        require_native,
        require_context,
        requirements_met,
        errors,
    }
}
