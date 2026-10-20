//! Review comment presentation. These helpers perform no PDF or model work.
use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::{AnnotationKind, DocumentComment, EditorAnnotation};

/// Materialize the current profile name at save/export boundaries. Display
/// overlays alone must not leave the PDF's standard author field out of date.
pub fn normalize_comment_authors(
    annotations: &mut [EditorAnnotation],
    author_id: &str,
    display_name: &str,
) -> usize {
    if author_id.is_empty() || display_name.trim().is_empty() {
        return 0;
    }
    let mut changed = 0;
    for annotation in annotations {
        if let AnnotationKind::Comment {
            author,
            author_id: stored_id,
            ..
        } = &mut annotation.kind
            && *stored_id == author_id
            && *author != display_name
        {
            *author = display_name.to_owned();
            changed += 1;
        }
    }
    changed
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewMarginMode {
    #[default]
    Footnotes,
    Comments,
}

/// An explicit profile ID survives name changes. Imported author names are
/// normalized only for display grouping; their original PDF values are kept.
pub fn comment_author_key(comment: &DocumentComment) -> String {
    if let Some(id) = comment.author_id.as_deref().filter(|id| !id.is_empty()) {
        return format!("id:{id}");
    }
    let name = comment
        .author
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if name.is_empty() {
        "anonymous".to_owned()
    } else {
        format!("name:{name}")
    }
}

fn author_hash(key: &str) -> u64 {
    key.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

/// A stable muted color with enough saturation to distinguish comment authors.
pub fn author_color(author_key: &str) -> [f32; 3] {
    color_for_hue((author_hash(author_key) % 3600) as f32 / 3600.0)
}

fn luminance(rgb: [f32; 3]) -> f32 {
    let linear = rgb.map(|channel| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

/// Retain each author's hue while making small name text readable on the
/// current Review theme. Border accents and original PDF colors are unchanged.
pub fn author_text_color_on(accent: [f32; 3], background: [f32; 3]) -> [f32; 3] {
    let background_luminance = luminance(background);
    let contrast = |rgb| {
        let foreground = luminance(rgb);
        (foreground.max(background_luminance) + 0.05)
            / (foreground.min(background_luminance) + 0.05)
    };
    if contrast(accent) >= 4.6 {
        return accent;
    }
    let endpoint = if background_luminance > 0.179 {
        0.0
    } else {
        1.0
    };
    let mix = |amount: f32| accent.map(|channel| channel * (1.0 - amount) + endpoint * amount);
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..12 {
        let middle = (low + high) * 0.5;
        if contrast(mix(middle)) >= 4.6 {
            high = middle;
        } else {
            low = middle;
        }
    }
    mix(high)
}

fn color_for_hue(hue: f32) -> [f32; 3] {
    let value = 0.78;
    let chroma = 0.48;
    let h = hue.rem_euclid(1.0) * 6.0;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let rgb = match h as usize {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    rgb.map(|channel| channel + value - chroma)
}

/// Resolve nearby hues deterministically for the current document. Sorted
/// author keys make colors independent of annotation or reply ordering.
pub fn author_colors(comments: &[DocumentComment]) -> HashMap<String, [f32; 3]> {
    let keys = comments
        .iter()
        .map(comment_author_key)
        .collect::<BTreeSet<_>>();
    let minimum_gap = (0.06_f32).min(0.5 / keys.len().max(1) as f32);
    let mut hues: Vec<f32> = Vec::with_capacity(keys.len());
    keys.into_iter()
        .map(|key| {
            let mut hue = (author_hash(&key) % 3600) as f32 / 3600.0;
            for _ in 0..3600 {
                if hues.iter().all(|old| {
                    let difference = (hue - old).abs();
                    difference.min(1.0 - difference) >= minimum_gap
                }) {
                    break;
                }
                hue = (hue + 0.381_966).fract();
            }
            hues.push(hue);
            (key, color_for_hue(hue))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DocumentCommentSource, PdfRect};

    fn comment(name: &str, id: Option<&str>) -> DocumentComment {
        DocumentComment {
            page_index: 0,
            anchor: (0.0, 0.0),
            rect: PdfRect::new(0.0, 0.0, 1.0, 1.0),
            text: "A note".to_owned(),
            author: Some(name.to_owned()),
            author_id: id.map(str::to_owned),
            color_rgb: [1.0, 0.0, 0.0],
            source: DocumentCommentSource::Embedded,
            is_reply: false,
            reply_to: None,
            key: name.to_owned(),
        }
    }

    #[test]
    fn author_colors_survive_rename_and_comment_reordering() {
        let first = comment("Sarah", Some("profile-a"));
        let second = comment("Reader", None);
        let colors = author_colors(&[first.clone(), second.clone()]);
        let mut renamed = first.clone();
        renamed.author = Some("Sarah Lawsky".to_owned());
        assert_eq!(colors, author_colors(&[second.clone(), renamed]));
        assert_ne!(
            colors[&comment_author_key(&first)],
            colors[&comment_author_key(&second)]
        );
        assert_eq!(first.color_rgb, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn anonymous_or_foreign_comments_do_not_claim_this_profile() {
        assert!(!comment("LawPDF", None).is_owned_by("profile-a"));
        assert!(!comment("Same display name", Some("profile-b")).is_owned_by("profile-a"));
        assert!(comment("Renamed", Some("profile-a")).is_owned_by("profile-a"));
        assert!(!comment("", Some("")).is_owned_by(""));
    }

    #[test]
    fn author_names_have_readable_contrast_in_all_review_themes() {
        for background in [
            [1.0, 0.992, 0.973],
            [0.98, 0.949, 0.878],
            [0.133, 0.157, 0.204],
        ] {
            for hue in 0..72 {
                let foreground = author_text_color_on(color_for_hue(hue as f32 / 72.0), background);
                let a = luminance(foreground);
                let b = luminance(background);
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            }
        }
    }
}
