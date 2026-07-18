//! LiquidVision nano (LmV tier): embedded YOLO region detector run via pure-Rust
//! `tract`. Produces per-line vision features that mirror the Python sidecar
//! (`tools/lm2_liquidvision_feature_sidecar.py` + `apply_liquidvision_features`
//! in `tools/lm2_tabular_feature_dump.py`) so a CatBoost trained on those
//! features transfers to this runtime.
//!
//! Detections come out of the net in 512-letterbox pixel space. We un-letterbox
//! to fitz page-point space (y-DOWN from the top), then flip to PDF y-UP
//! (origin bottom-left) to match `DeepLiquidSourceLine` geometry.

use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use std::time::UNIX_EPOCH;
use tract_onnx::prelude::*;

use crate::liquid::DeepLiquidSourceLine;
use crate::pdf_backend::PdfEngine;

/// 9.25 MB nano packed into the binary (LmV opt-in tier).
const NANO_ONNX: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/profile-models/liquidvision/nano-epoch51.onnx"
));

const IMGSZ: usize = 512;
const CONF: f32 = 0.25;

#[derive(Debug, Clone, Copy)]
struct LetterboxTransform {
    scale: f64,
    pad_x: i64,
    pad_y: i64,
}

fn letterbox_transform(
    img_w: usize,
    img_h: usize,
    page_w_pts: f64,
    page_h_pts: f64,
) -> LetterboxTransform {
    LetterboxTransform {
        scale: (IMGSZ as f64 / page_w_pts).min(IMGSZ as f64 / page_h_pts),
        // floor((imgsz - rendered)/2) — matches the Python letterbox.
        pad_x: ((IMGSZ as i64 - img_w as i64) as f64 / 2.0).floor() as i64,
        pad_y: ((IMGSZ as i64 - img_h as i64) as f64 / 2.0).floor() as i64,
    }
}

fn unletterbox_bbox(
    bbox: [f64; 4],
    transform: LetterboxTransform,
    page_w_pts: f64,
    page_h_pts: f64,
) -> Option<[f64; 4]> {
    let x0 = ((bbox[0] - transform.pad_x as f64) / transform.scale).max(0.0);
    let y0 = ((bbox[1] - transform.pad_y as f64) / transform.scale).max(0.0);
    let x1 = ((bbox[2] - transform.pad_x as f64) / transform.scale).min(page_w_pts);
    let y1 = ((bbox[3] - transform.pad_y as f64) / transform.scale).min(page_h_pts);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Class index order baked into the epoch51 head (matches sidecar CLASSES).
const CLASSES: [&str; 7] = [
    "footnote",
    "table",
    "figure",
    "body",
    "heading",
    "furniture",
    "frontmatter",
];

fn precedence(cls: &str) -> u8 {
    match cls {
        "footnote" => 0,
        "table" | "figure" => 1,
        "furniture" => 2,
        "heading" => 3,
        "body" => 4,
        _ => 5, // frontmatter
    }
}

fn route_for_class(cls: &str) -> &'static str {
    match cls {
        "table" | "figure" | "furniture" | "frontmatter" => "hide_noise",
        "footnote" => "marginalia",
        "body" | "heading" => "keep_veto",
        _ => "none",
    }
}

type RunModel = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// One region detection in fitz page-point space (y-DOWN from the top), i.e.
/// `y0` = top edge, `y1` = bottom edge, `y0 < y1`. This deliberately matches the
/// Python sidecar, which matches line geometry against detections WITHOUT a
/// y-orientation flip (lines are y-up); we replicate that exactly so the trained
/// CatBoost features transfer.
#[derive(Debug, Clone)]
pub struct LvDetection {
    pub class: &'static str,
    pub score: f32,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub area_norm: f64,
}

/// Per-line LiquidVision features, mirroring the sidecar/dump schema.
/// Stored on every `DeepLiquidSourceLine`; default = all-zero (Lm path / un-rendered).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct LvLineFeatures {
    pub class: String,
    pub route: String,
    pub score: f64,
    pub coverage: f64,
    pub region_area_norm: f64,
    pub page_region_count: f64,
    pub page_footnote_count: f64,
    pub page_table_figure_count: f64,
    pub footnote_score: f64,
    pub table_score: f64,
    pub figure_score: f64,
    pub body_score: f64,
    pub heading_score: f64,
    pub furniture_score: f64,
    pub frontmatter_score: f64,
    pub has_region: bool,
}

pub struct LiquidVision {
    model: RunModel,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiquidVisionFillReport {
    pub pages_attempted: usize,
    pub pages_filled: usize,
    pub pages_cached: usize,
    pub lines_filled: usize,
    pub elapsed_ms: f64,
    pub errors: Vec<String>,
}

pub fn liquidvision_enabled(default_enabled: bool) -> bool {
    std::env::var("LAWPDF_LMV")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(default_enabled)
}

pub fn fill_document_features(
    engine: &PdfEngine,
    path: &Path,
    page_count: usize,
    lines: &mut [DeepLiquidSourceLine],
) -> Result<LiquidVisionFillReport> {
    let started = Instant::now();
    let lv = LiquidVision::global().ok_or_else(|| anyhow::anyhow!("LiquidVision unavailable"))?;
    let mut by_page: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        by_page.entry(line.page_index).or_default().push(index);
    }
    let pages_attempted = by_page.len();
    let mut pages_filled = 0usize;
    let mut pages_cached = 0usize;
    let mut lines_filled = 0usize;
    let mut errors = Vec::new();
    for (page_index, line_indices) in by_page {
        if page_index >= page_count {
            errors.push(format!(
                "page {} is outside document bounds",
                page_index + 1
            ));
            continue;
        }
        let cache_key = liquidvision_cache_key(path, page_index);
        let cached = cache_key.as_ref().and_then(liquidvision_cached_detections);
        let detections = if let Some(detections) = cached {
            pages_cached += 1;
            detections
        } else {
            let page = match engine.render_page_for_vision(path, page_index, IMGSZ as u32) {
                Ok(page) => page,
                Err(error) => {
                    errors.push(format!("page {} render failed: {error}", page_index + 1));
                    continue;
                }
            };
            let detections = match lv.detect_page(
                &page.rgb,
                page.width,
                page.height,
                page.page_width_pts,
                page.page_height_pts,
            ) {
                Ok(detections) => detections,
                Err(error) => {
                    errors.push(format!("page {} inference failed: {error}", page_index + 1));
                    continue;
                }
            };
            if let Some(cache_key) = cache_key {
                liquidvision_cache_detections(cache_key, &detections);
            }
            detections
        };
        let page_region_count = detections.len();
        let page_footnote_count = detections
            .iter()
            .filter(|detection| detection.class == "footnote")
            .count();
        let page_table_figure_count = detections
            .iter()
            .filter(|detection| matches!(detection.class, "table" | "figure"))
            .count();
        for index in line_indices {
            let line = &mut lines[index];
            line.lv = assign_line_features(
                line.left as f64,
                line.bottom as f64,
                line.right as f64,
                line.top as f64,
                &detections,
                page_region_count,
                page_footnote_count,
                page_table_figure_count,
            );
            lines_filled += 1;
        }
        pages_filled += 1;
    }
    Ok(LiquidVisionFillReport {
        pages_attempted,
        pages_filled,
        pages_cached,
        lines_filled,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        errors,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LiquidVisionCacheKey {
    path: PathBuf,
    file_len: u64,
    modified_nanos: u128,
    page_index: usize,
}

const LIQUIDVISION_CACHE_PAGE_CAPACITY: usize = 256;

fn liquidvision_detection_cache() -> &'static Mutex<BTreeMap<LiquidVisionCacheKey, Vec<LvDetection>>>
{
    static CACHE: OnceLock<Mutex<BTreeMap<LiquidVisionCacheKey, Vec<LvDetection>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn liquidvision_cache_key(path: &Path, page_index: usize) -> Option<LiquidVisionCacheKey> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(LiquidVisionCacheKey {
        path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        file_len: metadata.len(),
        modified_nanos,
        page_index,
    })
}

fn liquidvision_cached_detections(key: &LiquidVisionCacheKey) -> Option<Vec<LvDetection>> {
    liquidvision_detection_cache()
        .lock()
        .ok()?
        .get(key)
        .cloned()
}

fn liquidvision_cache_detections(key: LiquidVisionCacheKey, detections: &[LvDetection]) {
    let Ok(mut cache) = liquidvision_detection_cache().lock() else {
        return;
    };
    if cache.len() >= LIQUIDVISION_CACHE_PAGE_CAPACITY
        && let Some(oldest) = cache.keys().next().cloned()
    {
        cache.remove(&oldest);
    }
    cache.insert(key, detections.to_vec());
}

impl LiquidVision {
    /// Lazily loaded process-global instance (load+optimize is one-time, ~185ms).
    pub fn global() -> Option<&'static LiquidVision> {
        static G: OnceLock<Option<LiquidVision>> = OnceLock::new();
        G.get_or_init(|| match LiquidVision::load() {
            Ok(lv) => Some(lv),
            Err(err) => {
                eprintln!("[LmV] failed to load LiquidVision nano: {err}");
                None
            }
        })
        .as_ref()
    }

    fn load() -> Result<Self> {
        let model = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(NANO_ONNX))?
            .with_input_fact(0, f32::fact([1, 3, IMGSZ, IMGSZ]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { model })
    }

    /// Run the nano on an RGB page raster (row-major `img_h` x `img_w` x 3).
    /// `img_w`/`img_h` are the rendered pixels (≈ page_pts * scale, letterbox-ready).
    /// Returns detections in PDF y-UP page-point space.
    pub fn detect_page(
        &self,
        rgb: &[u8],
        img_w: usize,
        img_h: usize,
        page_w_pts: f64,
        page_h_pts: f64,
    ) -> Result<Vec<LvDetection>> {
        let transform = letterbox_transform(img_w, img_h, page_w_pts, page_h_pts);
        let pad_x = transform.pad_x;
        let pad_y = transform.pad_y;

        let plane = IMGSZ * IMGSZ;
        let mut chan = vec![114f32 / 255.0; 3 * plane];
        for y in 0..img_h {
            let cy = pad_y + y as i64;
            if cy < 0 || cy >= IMGSZ as i64 {
                continue;
            }
            for x in 0..img_w {
                let cx = pad_x + x as i64;
                if cx < 0 || cx >= IMGSZ as i64 {
                    continue;
                }
                let sidx = (y * img_w + x) * 3;
                let didx = cy as usize * IMGSZ + cx as usize;
                chan[didx] = rgb[sidx] as f32 / 255.0;
                chan[plane + didx] = rgb[sidx + 1] as f32 / 255.0;
                chan[2 * plane + didx] = rgb[sidx + 2] as f32 / 255.0;
            }
        }

        let input = Tensor::from_shape(&[1, 3, IMGSZ, IMGSZ], &chan)?;
        let result = self.model.run(tvec!(input.into()))?;
        let out = result[0].to_array_view::<f32>()?; // [1, 300, 6]

        let page_area = (page_w_pts * page_h_pts).max(1.0);
        let mut dets = Vec::new();
        for i in 0..300 {
            let score = out[[0, i, 4]];
            if score < CONF {
                continue;
            }
            let cls_index = out[[0, i, 5]].round() as i64;
            if cls_index < 0 || cls_index as usize >= CLASSES.len() {
                continue;
            }
            let Some([x0, y0, x1, y1]) = unletterbox_bbox(
                [
                    out[[0, i, 0]] as f64,
                    out[[0, i, 1]] as f64,
                    out[[0, i, 2]] as f64,
                    out[[0, i, 3]] as f64,
                ],
                transform,
                page_w_pts,
                page_h_pts,
            ) else {
                continue;
            };
            let area_norm = ((x1 - x0) * (y1 - y0)) / page_area;
            dets.push(LvDetection {
                class: CLASSES[cls_index as usize],
                score,
                x0,
                y0,
                x1,
                y1,
                area_norm,
            });
        }
        Ok(dets)
    }
}

fn intersection_area(
    ax0: f64,
    ay0: f64,
    ax1: f64,
    ay1: f64,
    bx0: f64,
    by0: f64,
    bx1: f64,
    by1: f64,
) -> f64 {
    let x0 = ax0.max(bx0);
    let y0 = ay0.max(by0);
    let x1 = ax1.min(bx1);
    let y1 = ay1.min(by1);
    (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
}

/// Assign per-line LiquidVision features from page detections, mirroring the
/// sidecar (center-in-box OR overlap>=0.35, class precedence, per-class best).
/// `line` bbox is PDF y-up (left, bottom, right, top). Page-level counts are passed in.
pub fn assign_line_features(
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
    dets: &[LvDetection],
    page_region_count: usize,
    page_footnote_count: usize,
    page_table_figure_count: usize,
) -> LvLineFeatures {
    let mut feat = LvLineFeatures {
        has_region: true, // sidecar emits a row for every line on a rendered page
        page_region_count: page_region_count as f64,
        page_footnote_count: page_footnote_count as f64,
        page_table_figure_count: page_table_figure_count as f64,
        class: "none".to_string(),
        route: "none".to_string(),
        ..Default::default()
    };

    // NOTE: the line bbox is y-up (bottom < top), the detection bbox is fitz
    // y-down (y0 = top edge < y1 = bottom edge). The Python sidecar matches them
    // directly WITHOUT a y-flip, treating the line as (x0=left, y0=bottom,
    // x1=right, y1=top). We replicate that exactly for feature parity.
    let cx = (left + right) / 2.0;
    let cy = (bottom + top) / 2.0;
    let line_area = ((right - left) * (top - bottom)).max(1.0);
    let (ly0, ly1) = (bottom, top); // line as (x0=left, y0=bottom, x1=right, y1=top)

    // Collect matches (center-in-box and/or overlap>=0.35), as the sidecar does.
    let mut matches: Vec<&LvDetection> = Vec::new();
    for det in dets {
        let center_in = det.x0 <= cx && cx <= det.x1 && det.y0 <= cy && cy <= det.y1;
        if center_in {
            matches.push(det);
        }
        let overlap =
            intersection_area(left, ly0, right, ly1, det.x0, det.y0, det.x1, det.y1) / line_area;
        if overlap >= 0.35 {
            matches.push(det);
        }
    }
    if matches.is_empty() {
        return feat;
    }

    // best per class by score
    let mut best_by_class: std::collections::HashMap<&str, &LvDetection> =
        std::collections::HashMap::new();
    for det in &matches {
        best_by_class
            .entry(det.class)
            .and_modify(|cur| {
                if det.score > cur.score {
                    *cur = det;
                }
            })
            .or_insert(det);
    }
    // selected = min by (precedence, -score)
    let best = best_by_class
        .values()
        .min_by(|a, b| {
            precedence(a.class).cmp(&precedence(b.class)).then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        })
        .copied()
        .unwrap();

    feat.class = best.class.to_string();
    feat.route = route_for_class(best.class).to_string();
    feat.score = best.score as f64;
    feat.region_area_norm = best.area_norm;
    feat.coverage = matches
        .iter()
        .map(|det| {
            intersection_area(left, ly0, right, ly1, det.x0, det.y0, det.x1, det.y1) / line_area
        })
        .fold(0.0, f64::max);

    for (cls, det) in &best_by_class {
        let s = det.score as f64;
        match *cls {
            "footnote" => feat.footnote_score = s,
            "table" => feat.table_score = s,
            "figure" => feat.figure_score = s,
            "body" => feat.body_score = s,
            "heading" => feat.heading_score = s,
            "furniture" => feat.furniture_score = s,
            "frontmatter" => feat.frontmatter_score = s,
            _ => {}
        }
    }
    feat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn letterbox_transform_matches_portrait_page_geometry() {
        let transform = letterbox_transform(396, 512, 612.0, 792.0);

        assert_close(transform.scale, 512.0 / 792.0);
        assert_eq!(transform.pad_x, 58);
        assert_eq!(transform.pad_y, 0);
    }

    #[test]
    fn unletterbox_bbox_round_trips_and_clamps_page_bounds() {
        let transform = letterbox_transform(396, 512, 612.0, 792.0);
        let full_page = [
            transform.pad_x as f64,
            transform.pad_y as f64,
            transform.pad_x as f64 + 612.0 * transform.scale,
            transform.pad_y as f64 + 792.0 * transform.scale,
        ];

        let [x0, y0, x1, y1] = unletterbox_bbox(full_page, transform, 612.0, 792.0).unwrap();
        assert_close(x0, 0.0);
        assert_close(y0, 0.0);
        assert_close(x1, 612.0);
        assert_close(y1, 792.0);

        assert_eq!(
            unletterbox_bbox([-20.0, -10.0, 700.0, 900.0], transform, 612.0, 792.0),
            Some([0.0, 0.0, 612.0, 792.0])
        );
        assert_eq!(
            unletterbox_bbox([0.0, 0.0, 1.0, 1.0], transform, 612.0, 792.0),
            None
        );
    }
}
