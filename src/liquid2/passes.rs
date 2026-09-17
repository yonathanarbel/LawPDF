//! Block-level `apply_*` passes run by the pipeline driver in a fixed order.

use super::*;

pub(super) fn apply_action_neutral_blocksplit(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) {
    if sources.is_empty() {
        return;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut refs = sources
        .iter()
        .flat_map(|source| source.lines.iter().cloned())
        .collect::<Vec<_>>();
    refs.sort_by_key(|line| (line.page_index, line.line_index));

    let mut rebuilt_blocks = Vec::new();
    let mut rebuilt_sources = Vec::new();
    let mut current_role: Option<LiquidBlockRole> = None;
    let mut current_refs: Vec<LiquidSourceLineRef> = Vec::new();
    let mut previous_ref: Option<LiquidSourceLineRef> = None;
    let mut previous_role: Option<LiquidBlockRole> = None;

    for line_ref in refs {
        let role = line_ref.role;
        if lm2_blocksplit_should_split(
            previous_ref.as_ref(),
            &line_ref,
            previous_role,
            role,
            &line_by_id,
        ) {
            flush_action_neutral_blocksplit(
                &mut rebuilt_blocks,
                &mut rebuilt_sources,
                &mut current_refs,
                current_role.unwrap_or(role),
            );
        }
        current_role = Some(role);
        previous_ref = Some(line_ref.clone());
        previous_role = Some(role);
        current_refs.push(line_ref);
    }
    if let Some(role) = current_role {
        flush_action_neutral_blocksplit(
            &mut rebuilt_blocks,
            &mut rebuilt_sources,
            &mut current_refs,
            role,
        );
    }
    if !rebuilt_blocks.is_empty() {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    }
}

/// Reflow passes can correctly bridge prose around interleaved footnotes but,
/// in doing so, can also recombine paragraph blocks that the first assembly
/// pass separated. At the stable final boundary, split only inside remaining
/// Paragraph/Abstract blocks and only where the original source geometry still
/// proves a paragraph start. Existing block text is sliced rather than rebuilt
/// so recovered callout sentinels remain byte-for-byte intact.
pub(super) fn apply_final_source_backed_paragraph_splits(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut source_by_block = std::mem::take(sources)
        .into_iter()
        .map(|source| (source.block_index, source.lines))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::new();
    let mut repaired = 0usize;

    for (old_index, block) in old_blocks.into_iter().enumerate() {
        let Some(refs) = source_by_block.remove(&old_index) else {
            rebuilt_blocks.push(block);
            continue;
        };
        if !matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Abstract
        ) || refs.len() < 2
        {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        }

        let mut splits = Vec::<(usize, usize)>::new();
        let mut search_from = 0usize;
        for ref_index in 1..refs.len() {
            let (Some(previous), Some(current)) = (
                refs[ref_index - 1]
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id)),
                refs[ref_index]
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id)),
            ) else {
                continue;
            };
            if !lm2_final_paragraph_boundary(previous, current)
                || lm2_strong_source_body_wrap(previous, current)
            {
                continue;
            }
            let needle = clean_lm2_line_text(&refs[ref_index].text);
            if needle.is_empty() || needle.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            let Some(relative) = block.text[search_from..].find(&needle) else {
                continue;
            };
            let position = search_from + relative;
            if position == 0
                || block.text[..position].ends_with(CALLOUT_START)
                || position <= splits.last().map(|split| split.0).unwrap_or(0)
            {
                continue;
            }
            splits.push((position, ref_index));
            search_from = position + needle.len();
        }

        if splits.is_empty() {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        }

        let mut boundaries = Vec::with_capacity(splits.len() + 2);
        boundaries.push((0usize, 0usize));
        boundaries.extend(splits);
        boundaries.push((block.text.len(), refs.len()));
        for (segment_index, pair) in boundaries.windows(2).enumerate() {
            let (text_start, ref_start) = pair[0];
            let (text_end, ref_end) = pair[1];
            let text = block.text[text_start..text_end].trim().to_owned();
            if text.is_empty() || ref_start == ref_end {
                continue;
            }
            let segment_refs = &refs[ref_start..ref_end];
            let segment_role = lm2_final_split_child_role(block.role, segment_refs, &line_by_id);
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: segment_role,
                text,
                label: (segment_index == 0).then(|| block.label.clone()).flatten(),
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: segment_refs.to_vec(),
            });
            if segment_index > 0 {
                repaired += 1;
            }
        }
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

/// Separate a complete body sentence that a late heading reflow fused onto an
/// outline heading. Both sides must be independent one-row source segments,
/// and the body side must already carry a Paragraph-like source role and
/// terminal sentence punctuation; wrapped headings therefore stay intact.
pub(super) fn lm2_final_caption_source_line(line: &DeepLiquidSourceLine) -> bool {
    !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.segment_block_table_like
        && !line.page_object_overlaps_image_bbox
        && !line.segment_block_furniture_like
}

/// Keep a wrapped Figure/Table caption together when the source rows have one
/// segment, style, and ordinary line rhythm. The next heading is protected by
/// the same-segment, font/style, left-edge, and vertical-gap requirements.
pub(super) fn apply_final_caption_continuation_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for target in 0..blocks.len().saturating_sub(1) {
            let donor = target + 1;
            if !lm2_explicit_numbered_table_figure_caption(&blocks[target].text)
                || !matches!(
                    blocks[target].role,
                    LiquidBlockRole::Caption
                        | LiquidBlockRole::Noise
                        | LiquidBlockRole::Table
                        | LiquidBlockRole::Paragraph
                )
                || !matches!(
                    blocks[donor].role,
                    LiquidBlockRole::Paragraph | LiquidBlockRole::Noise | LiquidBlockRole::Caption
                )
                || !blocks[donor]
                    .text
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(char::is_lowercase)
            {
                continue;
            }
            let (Some(target_position), Some(donor_position)) = (
                source_position.get(&target).copied(),
                source_position.get(&donor).copied(),
            ) else {
                continue;
            };
            let (Some(previous_ref), Some(current_ref)) = (
                sources[target_position].lines.last(),
                sources[donor_position].lines.first(),
            ) else {
                continue;
            };
            let (Some(previous), Some(current)) = (
                previous_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                current_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            let page_width = previous.page_width.max(current.page_width).max(1.0);
            let font_ratio = previous.font_height.max(current.font_height)
                / previous.font_height.min(current.font_height).max(1.0);
            if previous.page_index != current.page_index
                || previous.line_index.saturating_add(1) != current.line_index
                || previous.segment_block_id != current.segment_block_id
                || previous.segment_block_id == 0
                || !lm2_final_caption_source_line(previous)
                || !lm2_final_caption_source_line(current)
                || previous.bold != current.bold
                || previous.italic != current.italic
                || font_ratio > 1.10
                || (previous.left - current.left).abs() / page_width > 0.03
                || vertical_gap(previous, current) > 0.025
            {
                continue;
            }
            operation = Some((target, donor));
            break;
        }
        let Some((target, donor)) = operation else {
            break;
        };
        let donor_text = blocks[donor].text.clone();
        append_line(&mut blocks[target].text, &donor_text);
        blocks[target].role = LiquidBlockRole::Caption;
        blocks[target].label = None;
        lm2_merge_block_sources(target, donor, sources);
        lm2_remove_block(donor, blocks, sources);
        repaired += 1;
    }
    repaired
}

/// Make the PDF's explicit `Figure N:` rows authoritative at the final
/// assembly boundary. Earlier grouping can label an ordinary sentence such as
/// `Figure 2 illustrates ...` as a caption, hide a caption-only row as noise,
/// or fuse the small italic caption with the following body paragraph. The
/// source segment gives us a narrow, layout-backed correction: an explicit
/// caption row plus only its contiguous, same-segment, same-style rows.
pub(super) fn apply_final_source_backed_figure_caption_isolation(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    #[derive(Clone)]
    struct CaptionGroup {
        ids: Vec<String>,
        text: String,
    }

    let mut groups = Vec::<CaptionGroup>::new();
    for (anchor, _) in decoded.iter().filter(|(line, _)| {
        lm2_explicit_numbered_table_figure_caption(&line.text)
            && normalize_text(&line.text)
                .split_whitespace()
                .next()
                .is_some_and(|word| matches!(word, "figure" | "fig."))
    }) {
        let mut rows = decoded
            .iter()
            .map(|(line, _)| line)
            .filter(|line| {
                line.page_index == anchor.page_index
                    && line.segment_block_id == anchor.segment_block_id
                    && line.line_index >= anchor.line_index
                    && line.bold == anchor.bold
                    && line.italic == anchor.italic
                    && line.font_height.max(anchor.font_height)
                        / line.font_height.min(anchor.font_height).max(1.0)
                        <= 1.10
                    && lm2_final_caption_source_line(line)
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|line| line.line_index);

        let mut expected = anchor.line_index;
        let mut ids = Vec::new();
        let mut text = String::new();
        for line in rows {
            if line.line_index != expected
                || (!ids.is_empty() && lm2_explicit_numbered_table_figure_caption(&line.text))
            {
                break;
            }
            ids.push(line.id.clone());
            append_line(&mut text, &line.text);
            expected += 1;
        }
        if !ids.is_empty() && !text.is_empty() {
            groups.push(CaptionGroup { ids, text });
        }
    }
    if groups.is_empty() {
        return 0;
    }

    let group_by_id = groups
        .iter()
        .enumerate()
        .flat_map(|(group_index, group)| group.ids.iter().map(move |id| (id.as_str(), group_index)))
        .collect::<HashMap<_, _>>();
    let deep_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let ref_by_id = sources
        .iter()
        .flat_map(|source| source.lines.iter())
        .filter_map(|line| line.id.as_deref().map(|id| (id, line.clone())))
        .collect::<HashMap<_, _>>();
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<HashMap<_, _>>();

    let mut seen = HashSet::<usize>::new();
    let mut rebuilt_blocks = Vec::new();
    let mut rebuilt_sources = Vec::new();
    let mut repaired = 0usize;

    let push_segment = |rebuilt_blocks: &mut Vec<LiquidBlock>,
                        rebuilt_sources: &mut Vec<LiquidBlockSourceLines>,
                        role: LiquidBlockRole,
                        text: String,
                        lines: Vec<LiquidSourceLineRef>| {
        if text.trim().is_empty() {
            return;
        }
        let block_index = rebuilt_blocks.len();
        rebuilt_blocks.push(LiquidBlock {
            role,
            text,
            label: None,
        });
        if !lines.is_empty() {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
    };

    for (block_index, block) in blocks.iter().enumerate() {
        let refs = source_by_block
            .get(&block_index)
            .cloned()
            .unwrap_or_default();
        let contains_caption = refs.iter().any(|line| {
            line.id
                .as_deref()
                .is_some_and(|id| group_by_id.contains_key(id))
        });
        if !contains_caption {
            let mut cloned = block.clone();
            if cloned.role == LiquidBlockRole::Caption
                && lm2_numbered_table_figure_caption(&cloned.text)
                && !lm2_explicit_numbered_table_figure_caption(&cloned.text)
            {
                cloned.role = LiquidBlockRole::Paragraph;
                cloned.label = None;
                repaired += 1;
            }
            let new_index = rebuilt_blocks.len();
            rebuilt_blocks.push(cloned);
            if !refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index: new_index,
                    lines: refs,
                });
            }
            continue;
        }

        repaired += 1;
        let mut body_refs = Vec::<LiquidSourceLineRef>::new();
        let flush_body =
            |body_refs: &mut Vec<LiquidSourceLineRef>,
             rebuilt_blocks: &mut Vec<LiquidBlock>,
             rebuilt_sources: &mut Vec<LiquidBlockSourceLines>| {
                if body_refs.is_empty() {
                    return;
                }
                let mut text = String::new();
                for line in body_refs.iter() {
                    append_line(&mut text, &line.text);
                }
                let role = body_refs
                    .iter()
                    .find_map(|line| {
                        matches!(
                            line.role,
                            LiquidBlockRole::Paragraph
                                | LiquidBlockRole::ListItem
                                | LiquidBlockRole::Heading
                                | LiquidBlockRole::Subheading
                        )
                        .then_some(line.role)
                    })
                    .unwrap_or(LiquidBlockRole::Paragraph);
                push_segment(
                    rebuilt_blocks,
                    rebuilt_sources,
                    role,
                    text,
                    std::mem::take(body_refs),
                );
            };

        for line in refs {
            let group_index = line
                .id
                .as_deref()
                .and_then(|id| group_by_id.get(id).copied());
            if let Some(group_index) = group_index {
                flush_body(&mut body_refs, &mut rebuilt_blocks, &mut rebuilt_sources);
                if seen.insert(group_index) {
                    let group = &groups[group_index];
                    let caption_refs = group
                        .ids
                        .iter()
                        .filter_map(|id| ref_by_id.get(id.as_str()).cloned())
                        .collect::<Vec<_>>();
                    push_segment(
                        &mut rebuilt_blocks,
                        &mut rebuilt_sources,
                        LiquidBlockRole::Caption,
                        group.text.clone(),
                        caption_refs,
                    );
                }
                continue;
            }
            let keep_as_body = line.id.as_deref().is_some_and(|id| {
                deep_by_id
                    .get(id)
                    .is_some_and(|deep| lm2_final_caption_source_line(deep))
            });
            if keep_as_body {
                body_refs.push(line);
            }
        }
        flush_body(&mut body_refs, &mut rebuilt_blocks, &mut rebuilt_sources);
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_leading_body_callout_with_remainder(text: &str) -> Option<(u16, String)> {
    if let Some(marker) = leading_sentineled_marker(text) {
        let end = leading_callout_marker_end(text.trim_start())?;
        let remainder = text.trim_start()[end..].trim_start();
        return remainder
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
            .then(|| (marker, text.trim_start().to_owned()));
    }
    let (marker, digit_len) = leading_numeric_token_marker_with_len(text)?;
    let trimmed = text.trim_start();
    let remainder = trimmed[digit_len..].trim_start();
    remainder
        .chars()
        .next()
        .is_some_and(char::is_alphabetic)
        .then(|| {
            (
                marker,
                format!("{CALLOUT_START}{marker}{CALLOUT_END} {remainder}"),
            )
        })
}

pub(super) fn lm2_final_noise_bridge_body_line(line: &DeepLiquidSourceLine) -> bool {
    !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.segment_block_table_like
        && !line.page_object_overlaps_image_bbox
        && !line.page_object_hide_candidate
        && !line.segment_block_furniture_like
        && line.font_ratio_doc >= 0.84
        && line.text.chars().any(char::is_alphabetic)
}

/// Restore a body row hidden between an open paragraph and a same-baseline
/// leading callout fragment. The three consecutive rows must be one mixed
/// source segment in body typography, and the callout needs a distinct
/// accepted definition. A nearby path stroke alone is not furniture evidence.
pub(super) fn apply_final_noise_inline_callout_bridge(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let accepted_heads = sources
        .iter()
        .flat_map(|source| source.lines.iter())
        .flat_map(|line| line.note_markers.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut repaired = 0usize;

    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for target in 0..blocks.len().saturating_sub(2) {
            let middle = target + 1;
            let donor = target + 2;
            if !matches!(
                blocks[target].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || blocks[middle].role != LiquidBlockRole::Noise
                || !matches!(
                    blocks[donor].role,
                    LiquidBlockRole::Paragraph
                        | LiquidBlockRole::Lead
                        | LiquidBlockRole::Quote
                        | LiquidBlockRole::Marginalia
                        | LiquidBlockRole::Footnote
                )
                || !lm2_reflow_paragraph_is_visibly_open(&strip_callout_sentinels_lm2(
                    &blocks[target].text,
                ))
                || !blocks[middle]
                    .text
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(char::is_lowercase)
            {
                continue;
            }
            let Some((marker, donor_text)) =
                lm2_leading_body_callout_with_remainder(&blocks[donor].text)
            else {
                continue;
            };
            if !accepted_heads.contains(&marker) {
                continue;
            }
            let (Some(target_position), Some(middle_position), Some(donor_position)) = (
                source_position.get(&target).copied(),
                source_position.get(&middle).copied(),
                source_position.get(&donor).copied(),
            ) else {
                continue;
            };
            let (Some(previous_ref), Some(middle_ref), Some(current_ref)) = (
                sources[target_position].lines.last(),
                sources[middle_position].lines.first(),
                sources[donor_position].lines.first(),
            ) else {
                continue;
            };
            if sources[middle_position].lines.len() != 1 || sources[donor_position].lines.is_empty()
            {
                continue;
            }
            let (Some(previous), Some(middle_line), Some(current)) = (
                previous_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                middle_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                current_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            let font_max = previous
                .font_height
                .max(middle_line.font_height)
                .max(current.font_height);
            let font_min = previous
                .font_height
                .min(middle_line.font_height)
                .min(current.font_height)
                .max(1.0);
            if previous.page_index != middle_line.page_index
                || middle_line.page_index != current.page_index
                || previous.line_index.saturating_add(1) != middle_line.line_index
                || middle_line.line_index.saturating_add(1) != current.line_index
                || previous.segment_block_id == 0
                || previous.segment_block_id != middle_line.segment_block_id
                || middle_line.segment_block_id != current.segment_block_id
                || !previous.segment_block_shape.eq_ignore_ascii_case("mixed")
                || !lm2_final_noise_bridge_body_line(previous)
                || !lm2_final_noise_bridge_body_line(middle_line)
                || !lm2_final_noise_bridge_body_line(current)
                || !same_visual_baseline_fragment(middle_line, current)
                || font_max / font_min > 1.18
            {
                continue;
            }
            operation = Some((target, middle, donor, donor_text));
            break;
        }

        let Some((target, middle, donor, donor_text)) = operation else {
            break;
        };
        let middle_text = blocks[middle].text.clone();
        append_line(&mut blocks[target].text, &middle_text);
        append_line(&mut blocks[target].text, &donor_text);
        blocks[target].role = LiquidBlockRole::Paragraph;
        blocks[target].label = None;
        lm2_merge_block_sources(target, middle, sources);
        lm2_remove_block(middle, blocks, sources);
        lm2_merge_block_sources(target, middle, sources);
        lm2_remove_block(middle, blocks, sources);
        if let Some(source) = sources
            .iter_mut()
            .find(|source| source.block_index == target)
        {
            for line in &mut source.lines {
                line.role = LiquidBlockRole::Paragraph;
                line.note_markers.clear();
            }
        }
        let _ = donor;
        repaired += 1;
    }
    repaired
}

pub(super) fn apply_final_fused_heading_body_splits(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut source_by_block = std::mem::take(sources)
        .into_iter()
        .map(|source| (source.block_index, source.lines))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::new();
    let mut repaired = 0usize;

    for (old_index, block) in old_blocks.into_iter().enumerate() {
        let Some(refs) = source_by_block.remove(&old_index) else {
            rebuilt_blocks.push(block);
            continue;
        };
        let split = if matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) && refs.len() >= 2
            && lm2_heading_starts_new_outline_item(&block.text)
        {
            (1..refs.len()).find_map(|ref_index| {
                let before = &refs[..ref_index];
                let after = &refs[ref_index..];
                if !before.iter().all(|line| {
                    matches!(
                        line.role,
                        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                    )
                }) || !after.iter().all(|line| {
                    matches!(
                        line.role,
                        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
                    )
                }) {
                    return None;
                }
                let previous = before
                    .last()?
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())?;
                let current = after
                    .first()?
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())?;
                if previous.page_index != current.page_index
                    || previous.line_index.checked_add(1) != Some(current.line_index)
                    || previous.segment_block_id == current.segment_block_id
                    || previous.segment_block_line_count != 1
                    || current.segment_block_line_count != 1
                    || !lm2_source_backed_body_display_line(current)
                    || !lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(
                        &current.text,
                    ))
                    || !current
                        .text
                        .chars()
                        .find(|ch| ch.is_alphabetic())
                        .is_some_and(char::is_uppercase)
                    || vertical_gap(previous, current) > 0.040
                {
                    return None;
                }
                let needle = clean_lm2_line_text(&refs[ref_index].text);
                let position = (!needle.is_empty())
                    .then(|| block.text.find(&needle))
                    .flatten()?;
                (position > 0).then_some((position, ref_index))
            })
        } else {
            None
        };

        let Some((text_position, ref_position)) = split else {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        };

        let heading_text = block.text[..text_position].trim().to_owned();
        let body_text = block.text[text_position..].trim().to_owned();
        if heading_text.is_empty() || body_text.is_empty() {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        }
        let heading_index = rebuilt_blocks.len();
        rebuilt_blocks.push(LiquidBlock {
            role: block.role,
            text: heading_text,
            label: block.label,
        });
        rebuilt_sources.push(LiquidBlockSourceLines {
            block_index: heading_index,
            lines: refs[..ref_position].to_vec(),
        });
        let body_index = rebuilt_blocks.len();
        rebuilt_blocks.push(LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: body_text,
            label: None,
        });
        rebuilt_sources.push(LiquidBlockSourceLines {
            block_index: body_index,
            lines: refs[ref_position..].to_vec(),
        });
        repaired += 1;
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_final_split_child_role(
    fallback: LiquidBlockRole,
    refs: &[LiquidSourceLineRef],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> LiquidBlockRole {
    let physical_footnote_rows = !refs.is_empty()
        && refs.iter().all(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(lm2_hard_physical_footnote_line)
        });
    if physical_footnote_rows {
        return LiquidBlockRole::Marginalia;
    }

    let hard_body_rows = !refs.is_empty()
        && refs.iter().all(|line| {
            matches!(
                line.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || line
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(|line| {
                    lm2_hard_body_display_line(line) || lm2_source_backed_body_display_line(line)
                })
        });
    hard_body_rows
        .then_some(LiquidBlockRole::Paragraph)
        .unwrap_or(fallback)
}

pub(super) fn lm2_hard_physical_footnote_line(line: &DeepLiquidSourceLine) -> bool {
    line.in_footnote_zone
        && line.segment_block_footnote_like
        && line.segment_block_shape.eq_ignore_ascii_case("footnote")
        && line.font_ratio_doc <= 0.94
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
}

/// Reassert the role of body displays that late note ownership can demote.
/// The two accepted shapes are deliberately structural: a statutory
/// subdivision between its colon lead-in and numbered children, or an
/// indented quotation between a colon lead-in and the page's first real note.
pub(super) fn apply_final_displayed_body_role_rescue(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_by_block = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let owner_by_coordinate = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().map(move |line| {
                (
                    (line.page_index, line.line_index),
                    (source.block_index, line),
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let mut operations = Vec::new();

    for block_index in 1..blocks.len().saturating_sub(1) {
        if !matches!(
            blocks[block_index].role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
        ) {
            continue;
        }
        let (Some(previous_position), Some(current_position), Some(next_position)) = (
            source_by_block.get(&(block_index - 1)).copied(),
            source_by_block.get(&block_index).copied(),
            source_by_block.get(&(block_index + 1)).copied(),
        ) else {
            continue;
        };
        let previous_refs = &sources[previous_position].lines;
        let current_refs = &sources[current_position].lines;
        let next_refs = &sources[next_position].lines;
        let (Some(previous_ref), Some(first_ref), Some(last_ref), Some(next_ref)) = (
            previous_refs.last(),
            current_refs.first(),
            current_refs.last(),
            next_refs.first(),
        ) else {
            continue;
        };
        let (Some(previous), Some(first), Some(last), Some(next)) = (
            previous_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            first_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            last_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            next_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
        ) else {
            continue;
        };
        if previous.page_index != first.page_index
            || first.page_index != last.page_index
            || previous.line_index.checked_add(1) != Some(first.line_index)
            || last.line_index.checked_add(1) != Some(next.line_index)
            || previous.segment_block_id == 0
            || current_refs
                .iter()
                .any(|line| !line.note_markers.is_empty())
            || !(lm2_hard_body_display_line(previous)
                || lm2_source_backed_body_display_line(previous))
            || current_refs.iter().any(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                    .is_none_or(|deep| {
                        !(lm2_hard_body_display_line(deep)
                            || lm2_source_backed_body_display_line(deep))
                            || deep.page_index != first.page_index
                            || deep.segment_block_id != previous.segment_block_id
                    })
            })
        {
            continue;
        }

        let statutory = blocks[block_index - 1].role == LiquidBlockRole::Paragraph
            && blocks[block_index + 1].role == LiquidBlockRole::Paragraph
            && blocks[block_index - 1].text.trim_end().ends_with(':')
            && lm2_parenthesized_alpha_lead(&blocks[block_index].text)
            && lm2_parenthesized_numeric_lead(&blocks[block_index + 1].text)
            && (lm2_hard_body_display_line(next) || lm2_source_backed_body_display_line(next))
            && next.segment_block_id == previous.segment_block_id
            && first.font_height <= previous.font_height * 0.92;

        let following_is_numbered_note = owner_by_coordinate
            .get(&(last.page_index, last.line_index.saturating_add(1)))
            .is_some_and(|(owner, line)| {
                matches!(
                    blocks.get(*owner).map(|block| block.role),
                    Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                ) && !line.note_markers.is_empty()
            });
        let page_width = first.page_width.max(previous.page_width).max(1.0);
        let quote = blocks[block_index - 1].role == LiquidBlockRole::Paragraph
            && blocks[block_index - 1].text.trim_end().ends_with(':')
            && following_is_numbered_note
            && (first.left - previous.left) / page_width > 0.02
            && first.font_height <= previous.font_height * 0.92
            && blocks[block_index]
                .text
                .chars()
                .find(|ch| ch.is_alphabetic())
                .is_some_and(char::is_uppercase);

        if statutory {
            operations.push((block_index, current_position, LiquidBlockRole::Paragraph));
        } else if quote {
            operations.push((block_index, current_position, LiquidBlockRole::Quote));
        }
    }

    for (block_index, source_position, role) in &operations {
        blocks[*block_index].role = *role;
        blocks[*block_index].label = None;
        for line in &mut sources[*source_position].lines {
            line.role = *role;
            line.note_markers.clear();
        }
    }
    operations.len()
}

pub(super) fn lm2_hard_body_display_line(line: &DeepLiquidSourceLine) -> bool {
    line.segment_block_id != 0
        && line.segment_block_shape.eq_ignore_ascii_case("body")
        && !line.segment_block_footnote_like
        && !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.doc_footnote_state
        && !line.doc_footnote_continuation
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
}

/// Remove only leading source rows that are independently proven page
/// furniture. This handles production slugs in their own block and journal
/// running heads fused to the first real body row without rebuilding body text
/// or disturbing recovered callout sentinels.
pub(super) fn apply_final_mixed_furniture_prefix_suppression(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_position = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let mut operations = Vec::new();

    for block_index in 0..blocks.len() {
        if !matches!(
            blocks[block_index].role,
            LiquidBlockRole::Paragraph
                | LiquidBlockRole::Lead
                | LiquidBlockRole::Quote
                | LiquidBlockRole::Heading
                | LiquidBlockRole::Subheading
                | LiquidBlockRole::Noise
        ) {
            continue;
        }
        let Some(position) = source_position.get(&block_index).copied() else {
            continue;
        };
        let refs = &sources[position].lines;
        let prefix_count = refs
            .iter()
            .take_while(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                    .is_some_and(lm2_final_hard_furniture_line)
            })
            .count();
        if prefix_count == 0 {
            continue;
        }
        if prefix_count == refs.len() {
            operations.push((block_index, position, prefix_count, None, false));
            continue;
        }
        let mut remainder = blocks[block_index].text.trim_start();
        let mut stripped = 0usize;
        for line in refs.iter().take(prefix_count) {
            let cleaned = clean_lm2_line_text(&line.text);
            let Some(next) = remainder.strip_prefix(cleaned.trim()) else {
                break;
            };
            remainder = next.trim_start();
            stripped += 1;
        }
        if stripped > 0 && !remainder.is_empty() {
            let salvage_body = blocks[block_index].role == LiquidBlockRole::Noise
                && refs[stripped..].iter().all(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied())
                        .is_some_and(lm2_hard_body_display_line)
                });
            operations.push((
                block_index,
                position,
                stripped,
                Some(remainder.to_owned()),
                salvage_body,
            ));
        }
    }

    for (block_index, position, count, replacement, salvage_body) in &operations {
        if let Some(replacement) = replacement {
            blocks[*block_index].text = replacement.clone();
            sources[*position].lines.drain(..*count);
            if *salvage_body {
                blocks[*block_index].role = LiquidBlockRole::Paragraph;
                blocks[*block_index].label = None;
                for line in &mut sources[*position].lines {
                    line.role = LiquidBlockRole::Paragraph;
                }
            }
        } else {
            blocks[*block_index].role = LiquidBlockRole::Noise;
            blocks[*block_index].label = None;
        }
    }
    operations.len()
}

pub(super) fn lm2_final_hard_furniture_line(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    lower == "footnote continued on next page"
        || looks_like_production_slug_boilerplate(&text)
        || (line.line_index <= 1
            && lower.contains("printer")
            && (lower.contains("delete") || lower.contains("proof")))
        || (line.line_index <= 2
            && looks_like_running_header(&lower)
            && !(lm2_hard_body_display_line(line)
                && attached_terminal_body_marker(&line.text).is_some())
            && (line.segment_block_furniture_like
                || line.segment_block_shape.eq_ignore_ascii_case("furniture")
                || line.doc_repeated_edge_text
                || line.role_hint.is_some_and(|role| {
                    matches!(
                        role,
                        LiquidBlockRole::Noise | LiquidBlockRole::Header | LiquidBlockRole::Footer
                    )
                })))
        || (line.doc_repeated_edge_text
            && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge))
        || (looks_like_page_label_furniture(&text)
            && !(lm2_hard_body_display_line(line)
                && attached_terminal_body_marker(&line.text).is_some())
            && (line.segment_block_furniture_like
                || line.segment_block_shape.eq_ignore_ascii_case("furniture")
                || line.doc_repeated_edge_text
                || line.role_hint.is_some_and(|role| {
                    matches!(
                        role,
                        LiquidBlockRole::Noise | LiquidBlockRole::Header | LiquidBlockRole::Footer
                    )
                })))
}

pub(super) fn lm2_final_paragraph_boundary(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    paragraph_boundary(previous, current)
        || lm2_displayed_lead_label_boundary(previous, current)
        || lm2_small_font_to_body_cliff_boundary(previous, current)
}

pub(super) fn lm2_lettered_list_item_start(text: &str) -> bool {
    let token = text
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let marker = token.trim_end_matches(['.', ')']);
    marker.len() == 1
        && marker.chars().all(|ch| ch.is_ascii_lowercase())
        && marker.len() != token.len()
}

pub(super) fn lm2_safe_mixed_segment_line(line: &DeepLiquidSourceLine) -> bool {
    line.segment_block_id != 0
        && line.segment_block_shape.eq_ignore_ascii_case("mixed")
        && !line.segment_block_footnote_like
        && !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
        && line.font_ratio_doc >= 0.90
}

/// Strong proof that two adjacent source rows are a single wrapped body flow,
/// even when PDF extraction reports a tall/overlapping row or positions a
/// leading inline footnote marker far to the right. The signed margin gate
/// excludes ordinary first-line paragraph indents and colon-led quotations.
pub(super) fn lm2_strong_source_body_wrap(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != current.page_index
        || previous.line_index.checked_add(1) != Some(current.line_index)
        || !lm2_reflow_starts_like_continuation(&current.text)
    {
        return false;
    }
    let page_width = previous.page_width.max(current.page_width).max(1.0);
    let signed_left_delta = (current.left - previous.left) / page_width;
    let font_ratio = previous.font_height.max(current.font_height)
        / previous.font_height.min(current.font_height).max(1.0);
    let ordinary_body_wrap = (lm2_hard_body_display_line(previous)
        || lm2_source_backed_body_display_line(previous))
        && (lm2_hard_body_display_line(current) || lm2_source_backed_body_display_line(current));
    let current_lower = current.text.trim_start().to_ascii_lowercase();
    let hanging_list_conjunction = previous.segment_block_id == current.segment_block_id
        && previous.segment_block_line_index.checked_add(1)
            == Some(current.segment_block_line_index)
        && lm2_safe_mixed_segment_line(previous)
        && lm2_safe_mixed_segment_line(current)
        && lm2_lettered_list_item_start(&previous.text)
        && (current_lower.starts_with("or ") || current_lower.starts_with("and "))
        && (0.0..=0.055).contains(&signed_left_delta);
    if !ordinary_body_wrap && !hanging_list_conjunction {
        return false;
    }
    let previous_open = previous.text.trim_end().ends_with('\u{0002}')
        || !lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text))
        || hanging_list_conjunction;
    if !previous_open || font_ratio > 1.20 {
        return false;
    }
    let right_indent_ceiling = if hanging_list_conjunction {
        0.055
    } else {
        PARAGRAPH_FIRST_LINE_INDENT
    };
    if signed_left_delta > right_indent_ceiling {
        return false;
    }
    let terminal_discretionary = previous.text.trim_end().ends_with('\u{0002}');
    let leading_inline_marker = leading_numeric_token_marker(&previous.text).is_some()
        || leading_sentineled_marker(&previous.text).is_some();
    let first_line_body_return = ordinary_body_wrap
        && previous.segment_block_id == current.segment_block_id
        && previous.segment_block_line_index == 0
        && current.segment_block_line_index == 1
        && signed_left_delta < 0.0;
    let outdent_floor = if terminal_discretionary || leading_inline_marker {
        -0.20
    } else if first_line_body_return {
        -0.08
    } else {
        -0.06
    };
    signed_left_delta >= outdent_floor && vertical_gap(previous, current) <= 0.036
}

pub(super) fn lm2_displayed_lead_label_boundary(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != current.page_index
        || previous.segment_block_id != current.segment_block_id
        || !lm2_source_backed_body_display_line(previous)
        || !lm2_source_backed_body_display_line(current)
        || current.page_width <= 0.0
        || current.left / current.page_width < 0.14
    {
        return false;
    }
    lm2_displayed_lead_label_text(&current.text)
        && (current.font_ratio_doc <= 0.98 || current.font_height <= previous.font_height * 0.95)
}

pub(super) fn lm2_displayed_lead_label_text(text: &str) -> bool {
    let text = collapse_whitespace(text);
    let Some(colon) = text.find(':') else {
        return false;
    };
    if colon > 120 {
        return false;
    }
    let label = text[..colon].trim();
    let remainder = text[colon + 1..].trim_start();
    let label_words = label.split_whitespace().collect::<Vec<_>>();
    (2..=14).contains(&label_words.len())
        && label_words.iter().all(|word| {
            let word = word.trim_matches(|ch: char| !ch.is_alphabetic());
            word.is_empty()
                || matches!(
                    word.to_ascii_lowercase().as_str(),
                    "a" | "an" | "and" | "as" | "for" | "in" | "of" | "or" | "the" | "to"
                )
                || word.chars().next().is_some_and(char::is_uppercase)
        })
        && remainder
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
}

pub(super) fn lm2_cross_page_displayed_lead_label_boundary(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    current.page_index == previous.page_index.saturating_add(1)
        && current.line_index <= 3
        && current.segment_block_line_index == 0
        && lm2_source_backed_body_display_line(current)
        && lm2_displayed_lead_label_text(&current.text)
        && current.font_ratio_doc <= 0.98
}

pub(super) fn lm2_small_font_to_body_cliff_boundary(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    previous.page_index == current.page_index
        && previous.segment_block_id == current.segment_block_id
        && lm2_body_segment_line(previous)
        && lm2_body_segment_line(current)
        && current.font_height >= previous.font_height * 1.07
        && previous.font_ratio_doc <= 0.97
        && current.font_ratio_doc >= 0.96
        && vertical_gap(previous, current) <= 0.025
        && lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text))
        && current
            .text
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
}

/// Split a merged run when the first source row on the next page is itself a
/// source-backed displayed label. This exposes the independent display before
/// source-order repair, without treating an ordinary page break as a split.
pub(super) fn apply_final_cross_page_display_run_splits(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut source_by_block = std::mem::take(sources)
        .into_iter()
        .map(|source| (source.block_index, source.lines))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::new();
    let mut repaired = 0usize;

    for (old_index, block) in old_blocks.into_iter().enumerate() {
        let Some(refs) = source_by_block.remove(&old_index) else {
            rebuilt_blocks.push(block);
            continue;
        };
        if !matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        ) || refs.len() < 2
        {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        }

        let mut splits = Vec::<(usize, usize)>::new();
        let mut search_from = 0usize;
        for ref_index in 1..refs.len() {
            let (Some(previous), Some(current)) = (
                refs[ref_index - 1]
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                refs[ref_index]
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            if !lm2_cross_page_displayed_lead_label_boundary(previous, current) {
                continue;
            }
            let needle = clean_lm2_line_text(&refs[ref_index].text);
            let Some(relative) = (!needle.is_empty())
                .then(|| block.text[search_from..].find(&needle))
                .flatten()
            else {
                continue;
            };
            let position = search_from + relative;
            if position == 0 {
                continue;
            }
            splits.push((position, ref_index));
            search_from = position + needle.len();
        }

        if splits.is_empty() {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        }

        let mut boundaries = vec![(0usize, 0usize)];
        boundaries.extend(splits);
        boundaries.push((block.text.len(), refs.len()));
        for pair in boundaries.windows(2) {
            let (text_start, ref_start) = pair[0];
            let (text_end, ref_end) = pair[1];
            let text = block.text[text_start..text_end].trim().to_owned();
            if text.is_empty() || ref_start == ref_end {
                continue;
            }
            let segment_refs = refs[ref_start..ref_end].to_vec();
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: lm2_final_split_child_role(block.role, &segment_refs, &line_by_id),
                text,
                label: None,
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: segment_refs,
            });
        }
        repaired += 1;
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_inline_dash_noise_fragment(text: &str) -> bool {
    let cleaned = strip_callout_sentinels_lm2(text);
    let trimmed = cleaned.trim();
    let Some(remainder) = trimmed
        .strip_prefix('\u{2014}')
        .or_else(|| trimmed.strip_prefix('\u{2013}'))
    else {
        return false;
    };
    remainder.trim().is_empty()
        || word_count(remainder) <= 4
            && !remainder.chars().any(char::is_numeric)
            && remainder.chars().any(char::is_alphabetic)
}

pub(super) fn lm2_text_before_terminal_body_marker(text: &str) -> &str {
    let trimmed = text.trim_end();
    if trimmed.ends_with(CALLOUT_END)
        && let Some(start) = trimmed.rfind(CALLOUT_START)
    {
        return trimmed[..start].trim_end();
    }
    if let Some(marker) = attached_terminal_ascii_marker(trimmed) {
        let marker = marker.to_string();
        if let Some(prefix) = trimmed.strip_suffix(&marker) {
            return prefix.trim_end();
        }
    }
    trimmed
}

/// Recover an em/en-dash fragment that PDF geometry isolated as a one-row
/// Noise block at the right edge of a body line. The following lowercase row
/// must return sharply to the body margin, so ordinary dash separators and
/// genuine paragraph starts remain untouched.
pub(super) fn apply_final_inline_dash_fragment_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for middle in 1..blocks.len().saturating_sub(1) {
            let target = middle - 1;
            let donor = middle + 1;
            if !matches!(
                blocks[target].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || blocks[middle].role != LiquidBlockRole::Noise
                || !matches!(
                    blocks[donor].role,
                    LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
                )
                || !lm2_inline_dash_noise_fragment(&blocks[middle].text)
            {
                continue;
            }
            let (Some(target_position), Some(middle_position), Some(donor_position)) = (
                source_position.get(&target).copied(),
                source_position.get(&middle).copied(),
                source_position.get(&donor).copied(),
            ) else {
                continue;
            };
            let (Some(previous_ref), [fragment_ref], Some(current_ref)) = (
                sources[target_position].lines.last(),
                sources[middle_position].lines.as_slice(),
                sources[donor_position].lines.first(),
            ) else {
                continue;
            };
            let (Some(previous), Some(fragment), Some(current)) = (
                previous_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                fragment_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                current_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            let page_width = previous
                .page_width
                .max(fragment.page_width)
                .max(current.page_width)
                .max(1.0);
            let font_ratio = previous
                .font_height
                .max(fragment.font_height)
                .max(current.font_height)
                / previous
                    .font_height
                    .min(fragment.font_height)
                    .min(current.font_height)
                    .max(1.0);
            if previous.page_index != fragment.page_index
                || fragment.page_index != current.page_index
                || previous.line_index.checked_add(1) != Some(fragment.line_index)
                || fragment.line_index.checked_add(1) != Some(current.line_index)
                || !same_visual_row_fragment(previous, fragment)
                || (current.left - fragment.left) / page_width > -0.20
                || vertical_gap(fragment, current) > 0.020
                || font_ratio > 1.15
                || !lm2_reflow_paragraph_is_visibly_open(lm2_text_before_terminal_body_marker(
                    &blocks[target].text,
                ))
                || !lm2_reflow_starts_like_continuation(&current.text)
                || !lm2_final_flow_line_ref(previous_ref, previous)
                || !lm2_final_flow_line_ref(current_ref, current)
                || lm2_final_hard_furniture_line(fragment)
                || fragment.in_footnote_zone
                || fragment.below_footnote_divider
                || fragment.in_ruled_cell
                || fragment.ruled_row_membership_exact
                || fragment.page_table_column_like
                || fragment.page_object_overlaps_image_bbox
                || fragment.font_ratio_doc < 0.90
            {
                continue;
            }
            operation = Some((target, middle, donor));
            break;
        }

        let Some((target, middle, donor)) = operation else {
            break;
        };
        let fragment_text = blocks[middle].text.clone();
        let donor_text = blocks[donor].text.clone();
        append_line(&mut blocks[target].text, &fragment_text);
        append_line(&mut blocks[target].text, &donor_text);
        lm2_merge_block_sources(target, middle, sources);
        lm2_remove_block(middle, blocks, sources);
        lm2_merge_block_sources(target, middle, sources);
        lm2_remove_block(middle, blocks, sources);
        repaired += 1;
    }

    repaired
}

pub(super) fn lm2_body_sized_fragment_line(
    line_ref: &LiquidSourceLineRef,
    line: &DeepLiquidSourceLine,
) -> bool {
    matches!(
        line_ref.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
    ) && !line.segment_block_footnote_like
        && !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
        && line.font_ratio_doc >= 0.84
        && line.text.chars().any(char::is_alphabetic)
}

/// Join adjacent blocks that are actually consecutive horizontal fragments of
/// one physical row. A numeric superscript may be the middle fragment; a
/// body-sized continuation mislabeled Table is accepted only when its source
/// ref still says Paragraph-like. The touching-x/baseline proof excludes
/// ordinary columns, list rows, and vertically separated paragraphs.
pub(super) fn apply_final_same_baseline_fragment_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for target in 0..blocks.len().saturating_sub(1) {
            let donor = target + 1;
            if !matches!(
                blocks[target].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) {
                continue;
            }
            let (Some(target_position), Some(donor_position)) = (
                source_position.get(&target).copied(),
                source_position.get(&donor).copied(),
            ) else {
                continue;
            };
            let (Some(previous_ref), Some(current_ref)) = (
                sources[target_position].lines.last(),
                sources[donor_position].lines.first(),
            ) else {
                continue;
            };
            let (Some(previous), Some(current)) = (
                previous_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                current_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            let donor_marker = lm2_source_ref_is_standalone_callout(current_ref)
                .then(|| strip_callout_sentinels_lm2(&current_ref.text))
                .and_then(|text| text.trim().parse::<u16>().ok());
            let previous_marker = lm2_source_ref_is_standalone_callout(previous_ref);
            let body_donor = lm2_body_sized_fragment_line(current_ref, current);
            if donor_marker.is_none() && !body_donor {
                continue;
            }
            if !matches!(
                blocks[donor].role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::Noise
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::Footnote
                    | LiquidBlockRole::Table
            ) {
                continue;
            }

            let anchor = if previous_marker {
                sources[target_position]
                    .lines
                    .iter()
                    .rev()
                    .skip(1)
                    .find_map(|line_ref| {
                        line_ref
                            .id
                            .as_deref()
                            .and_then(|id| line_by_id.get(id).copied())
                            .filter(|line| lm2_body_sized_fragment_line(line_ref, line))
                    })
            } else {
                lm2_body_sized_fragment_line(previous_ref, previous).then_some(previous)
            };
            let Some(anchor) = anchor else {
                continue;
            };
            let row_proven = if donor_marker.is_some() {
                lm2_marker_can_attach_to_previous_line(current, previous)
            } else if previous_marker {
                let page_height = anchor.page_height.max(current.page_height).max(1.0);
                let page_width = anchor.page_width.max(current.page_width).max(1.0);
                let body_baseline = (anchor.bottom - current.bottom).abs() / page_height <= 0.003
                    && (anchor.top - current.top).abs() / page_height <= 0.004;
                let horizontally_contiguous = current.left >= previous.right - 2.0
                    && current.left <= previous.right + (page_width * 0.008).max(2.5);
                lm2_marker_can_attach_to_previous_line(previous, anchor)
                    && body_baseline
                    && horizontally_contiguous
            } else {
                same_visual_row_fragment(previous, current)
            };
            if !row_proven {
                continue;
            }
            if donor_marker.is_none() {
                let font_ratio = anchor.font_height.max(current.font_height)
                    / anchor.font_height.min(current.font_height).max(1.0);
                if font_ratio > 1.20 {
                    continue;
                }
            }
            operation = Some((target, donor, donor_marker));
            break;
        }

        let Some((target, donor, donor_marker)) = operation else {
            break;
        };
        if let Some(marker) = donor_marker {
            if attached_terminal_body_marker(&blocks[target].text) != Some(marker) {
                append_standalone_marker_to_line(&mut blocks[target].text, &marker.to_string());
            }
        } else {
            let donor_text = blocks[donor].text.clone();
            append_line(&mut blocks[target].text, &donor_text);
        }
        lm2_merge_block_sources(target, donor, sources);
        lm2_remove_block(donor, blocks, sources);
        repaired += 1;
    }

    repaired
}

/// Join adjacent body fragments only when source geometry proves they are one
/// physical flow. A fully inverted adjacent source interval may be swapped
/// only when each block's emitted text independently follows its own source
/// rows; source refs alone are never permission to reorder prose.
/// This runs after every role/split repair so no later pass can reintroduce an
/// out-of-order interval or strand a wrapped line behind an artificial block
/// boundary.
pub(super) fn apply_final_body_source_order_and_coalescing(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut repaired = apply_final_cross_page_display_run_splits(blocks, sources, decoded);
    repaired += apply_final_inline_dash_fragment_reflow(blocks, sources, decoded);
    repaired += apply_final_same_baseline_fragment_reflow(blocks, sources, decoded);
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let first_body_line_by_page = sources
        .iter()
        .filter(|source| {
            lm2_final_body_block_candidate(source.block_index, blocks, sources, &line_by_id)
        })
        .flat_map(|source| source.lines.iter())
        .filter_map(|line_ref| {
            let line = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())?;
            lm2_final_flow_line_ref(line_ref, line).then_some(line)
        })
        .fold(HashMap::<usize, usize>::new(), |mut first, line| {
            first
                .entry(line.page_index)
                .and_modify(|index| *index = (*index).min(line.line_index))
                .or_insert(line.line_index);
            first
        });
    // Correct an actually inverted adjacent body pair only when each emitted
    // block independently follows the text order of its own source rows. A
    // merged block can have sorted provenance even when its appended text was
    // reversed; the text-congruence gate deliberately rejects that shape.
    loop {
        let mut operation = None;
        for index in 0..blocks.len().saturating_sub(1) {
            if !lm2_final_body_block_candidate(index, blocks, sources, &line_by_id)
                || !lm2_final_body_block_candidate(index + 1, blocks, sources, &line_by_id)
                || !lm2_block_text_follows_source_rows(index, blocks, sources, &line_by_id)
                || !lm2_block_text_follows_source_rows(index + 1, blocks, sources, &line_by_id)
            {
                continue;
            }
            let (Some(left), Some(right)) = (
                lm2_body_source_interval(index, sources, &line_by_id),
                lm2_body_source_interval(index + 1, sources, &line_by_id),
            ) else {
                continue;
            };
            if left.0 > right.1 {
                let previous = line_by_id
                    .values()
                    .copied()
                    .find(|line| (line.page_index, line.line_index) == right.1);
                let current = line_by_id
                    .values()
                    .copied()
                    .find(|line| (line.page_index, line.line_index) == left.0);
                if !matches!((previous, current), (Some(previous), Some(current)) if
                    lm2_strong_source_body_wrap(previous, current)
                        || lm2_same_physical_body_segment_order(previous, current))
                {
                    continue;
                }
                operation = Some(index);
                break;
            }
        }
        let Some(index) = operation else {
            break;
        };
        blocks.swap(index, index + 1);
        for source in sources.iter_mut() {
            if source.block_index == index {
                source.block_index = index + 1;
            } else if source.block_index == index + 1 {
                source.block_index = index;
            }
        }
        repaired += 1;
    }

    // Resolve adjacent open lowercase wraps before the general transparent-gap
    // pass. This uses raw source text so a terminal U+0002 hyphen remains
    // available even after the assembled block text has discarded it.
    loop {
        let mut operation = None;
        for index in 0..blocks.len().saturating_sub(1) {
            if !matches!(
                blocks[index].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || !matches!(
                blocks[index + 1].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) {
                continue;
            }
            let Some((previous, current, _)) =
                lm2_body_edge_lines(index, index + 1, sources, &line_by_id)
            else {
                continue;
            };
            if lm2_strong_source_body_wrap(previous, current) {
                operation = Some((index, previous.text.trim_end().ends_with('\u{0002}')));
                break;
            }
        }
        let Some((target, source_discretionary_hyphen)) = operation else {
            break;
        };
        let donor = target + 1;
        let donor_text = blocks[donor].text.clone();
        if source_discretionary_hyphen && !blocks[target].text.trim_end().ends_with('-') {
            blocks[target].text.push('-');
        }
        append_line(&mut blocks[target].text, &donor_text);
        lm2_merge_block_sources(target, donor, sources);
        lm2_remove_block(donor, blocks, sources);
        repaired += 1;
    }

    // Fold a source-isolated superscript into the preceding prose before the
    // ordinary continuity pass. The donor prose still has to be in the same
    // physical body flow and must not cross a real paragraph boundary.
    loop {
        let mut operation = None;
        for index in 0..blocks.len().saturating_sub(2) {
            if blocks[index].role != LiquidBlockRole::Paragraph
                || blocks[index + 2].role != LiquidBlockRole::Paragraph
                || !lm2_standalone_callout_block(&blocks[index + 1].text)
            {
                continue;
            }
            let Some((previous, current, _)) =
                lm2_body_edge_lines(index, index + 2, sources, &line_by_id)
            else {
                continue;
            };
            if previous.page_index == current.page_index
                && previous.segment_block_id == current.segment_block_id
                && current.line_index > previous.line_index
                && !lm2_final_paragraph_boundary(previous, current)
            {
                operation = Some(index);
                break;
            }
        }
        let Some(index) = operation else {
            break;
        };
        let marker = blocks[index + 1].text.clone();
        if marker.chars().all(|ch| ch.is_ascii_digit()) {
            append_standalone_marker_to_line(&mut blocks[index].text, &marker);
        } else {
            blocks[index].text = format!("{}{}", blocks[index].text.trim_end(), marker.trim());
        }
        lm2_merge_block_sources(index, index + 1, sources);
        lm2_remove_block(index + 1, blocks, sources);
        repaired += 1;
    }

    loop {
        let mut operation = None;
        for target in 0..blocks.len().saturating_sub(1) {
            if !lm2_final_body_block_candidate(target, blocks, sources, &line_by_id) {
                continue;
            }
            let mut donor = None;
            for index in (target + 1)..blocks.len() {
                if lm2_final_body_block_candidate(index, blocks, sources, &line_by_id) {
                    donor = Some(index);
                    break;
                }
                if !matches!(
                    blocks[index].role,
                    LiquidBlockRole::Noise
                        | LiquidBlockRole::Header
                        | LiquidBlockRole::Footer
                        | LiquidBlockRole::Marginalia
                        | LiquidBlockRole::Footnote
                ) {
                    break;
                }
            }
            let Some(donor) = donor else {
                continue;
            };
            if !blocks[target + 1..donor].iter().all(|block| {
                matches!(
                    block.role,
                    LiquidBlockRole::Noise
                        | LiquidBlockRole::Header
                        | LiquidBlockRole::Footer
                        | LiquidBlockRole::Marginalia
                        | LiquidBlockRole::Footnote
                )
            }) {
                continue;
            }
            let Some((previous, current, terminal_callout_gap)) =
                lm2_body_edge_lines(target, donor, sources, &line_by_id)
            else {
                continue;
            };
            if lm2_proven_body_continuation(
                &blocks[target].text,
                &blocks[donor].text,
                previous,
                current,
                terminal_callout_gap,
                &first_body_line_by_page,
            ) {
                operation = Some((target, donor));
                break;
            }
        }
        let Some((target, donor)) = operation else {
            break;
        };
        let donor_text = blocks[donor].text.clone();
        append_line(&mut blocks[target].text, &donor_text);
        lm2_merge_block_sources(target, donor, sources);
        lm2_remove_block(donor, blocks, sources);
        repaired += 1;
    }

    repaired
}

pub(super) fn lm2_same_physical_body_segment_order(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    previous.page_index == current.page_index
        && previous.line_index < current.line_index
        && current.line_index.saturating_sub(previous.line_index) <= 8
        && previous.segment_block_id != 0
        && previous.segment_block_id == current.segment_block_id
        && lm2_source_backed_body_display_line(previous)
        && lm2_source_backed_body_display_line(current)
        && previous.font_height.max(current.font_height)
            / previous.font_height.min(current.font_height).max(1.0)
            <= 1.25
}

pub(super) fn lm2_body_source_interval(
    block_index: usize,
    sources: &[LiquidBlockSourceLines],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> Option<((usize, usize), (usize, usize))> {
    let source = sources
        .iter()
        .find(|source| source.block_index == block_index)?;
    let mut coordinates = source.lines.iter().filter_map(|line| {
        let deep = line
            .id
            .as_deref()
            .and_then(|id| line_by_id.get(id).copied())?;
        lm2_final_flow_line_ref(line, deep).then_some((deep.page_index, deep.line_index))
    });
    let first = coordinates.next()?;
    Some(
        coordinates.fold((first, first), |(minimum, maximum), coordinate| {
            (minimum.min(coordinate), maximum.max(coordinate))
        }),
    )
}

pub(super) fn lm2_block_text_follows_source_rows(
    block_index: usize,
    blocks: &[LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let Some(block) = blocks.get(block_index) else {
        return false;
    };
    let Some(source) = sources
        .iter()
        .find(|source| source.block_index == block_index)
    else {
        return false;
    };
    let haystack = lm2_text_order_tokens(&block.text);
    if haystack.is_empty() {
        return false;
    }
    let mut cursor = 0usize;
    let mut previous_coordinate = None;
    let mut matched_rows = 0usize;
    let mut comparable_rows = 0usize;
    for line in &source.lines {
        let Some(deep) = line
            .id
            .as_deref()
            .and_then(|id| line_by_id.get(id).copied())
            .filter(|deep| lm2_final_flow_line_ref(line, deep))
        else {
            continue;
        };
        let coordinate = (deep.page_index, deep.line_index);
        if previous_coordinate.is_some_and(|previous| coordinate <= previous) {
            return false;
        }
        previous_coordinate = Some(coordinate);
        let anchor = lm2_text_order_tokens(&deep.text)
            .into_iter()
            .take(4)
            .collect::<Vec<_>>();
        if anchor.len() < 2 {
            continue;
        }
        comparable_rows += 1;
        if anchor.len() > haystack.len().saturating_sub(cursor) {
            return false;
        }
        let Some(relative) = haystack[cursor..]
            .windows(anchor.len())
            .position(|window| window == anchor.as_slice())
        else {
            return false;
        };
        cursor += relative + anchor.len();
        matched_rows += 1;
    }
    comparable_rows > 0 && matched_rows == comparable_rows
}

pub(super) fn lm2_text_order_tokens(text: &str) -> Vec<String> {
    strip_callout_sentinels_lm2(text)
        .replace('\u{0002}', "")
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| !ch.is_alphanumeric());
            (!token.is_empty() && token.chars().any(char::is_alphabetic))
                .then(|| token.to_lowercase())
        })
        .collect()
}

pub(super) fn lm2_final_body_block_candidate(
    block_index: usize,
    blocks: &[LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let Some(block) = blocks.get(block_index) else {
        return false;
    };
    if matches!(
        block.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
    ) {
        return true;
    }
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }
    let Some(source) = sources
        .iter()
        .find(|source| source.block_index == block_index)
    else {
        return false;
    };
    let textual_continuation = block.text.trim_end().ends_with('-')
        || leading_sentineled_marker(&block.text).is_some()
        || lm2_reflow_starts_like_continuation(&block.text);
    let contains_body_flow = source.lines.iter().any(|line| {
        matches!(
            line.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        ) && line
            .id
            .as_deref()
            .and_then(|id| line_by_id.get(id).copied())
            .is_some_and(|deep| lm2_body_sized_source_flow_ref(line, deep))
    });
    let embedded_body_continuation = source.lines.as_slice().first().is_some_and(|line| {
        line.id
            .as_deref()
            .and_then(|id| line_by_id.get(id).copied())
            .is_some_and(|deep| {
                source.lines.len() == 1
                    && deep.segment_block_line_index > 0
                    && deep.segment_block_shape.eq_ignore_ascii_case("body")
                    && !deep.bold
                    && !deep.is_block_indented
                    && lm2_source_backed_body_display_line(deep)
                    && deep.font_ratio_doc >= 0.95
            })
    });
    (textual_continuation || contains_body_flow || embedded_body_continuation)
        && source.lines.iter().any(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(|deep| lm2_final_flow_line_ref(line, deep))
        })
}

pub(super) fn lm2_body_edge_lines<'a>(
    target: usize,
    donor: usize,
    sources: &'a [LiquidBlockSourceLines],
    line_by_id: &HashMap<&str, &'a DeepLiquidSourceLine>,
) -> Option<(&'a DeepLiquidSourceLine, &'a DeepLiquidSourceLine, bool)> {
    let target_source = sources.iter().find(|source| source.block_index == target)?;
    let donor_source = sources.iter().find(|source| source.block_index == donor)?;
    let previous = target_source
        .lines
        .iter()
        .filter_map(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .filter(|deep| lm2_final_flow_line_ref(line, deep))
        })
        .max_by_key(|line| (line.page_index, line.line_index))?;
    let terminal_callout_gap = target_source.lines.iter().any(|line| {
        line.page_index == previous.page_index
            && line.line_index == previous.line_index.saturating_add(1)
            && lm2_source_ref_is_standalone_callout(line)
    }) || sources
        .iter()
        .filter(|source| source.block_index > target && source.block_index < donor)
        .flat_map(|source| source.lines.iter())
        .any(|line| {
            line.page_index == previous.page_index
                && line.line_index == previous.line_index.saturating_add(1)
                && lm2_source_ref_is_standalone_callout(line)
        })
        || line_by_id.values().copied().any(|line| {
            line.page_index == previous.page_index
                && line.line_index == previous.line_index.saturating_add(1)
                && lm2_standalone_callout_block(&line.text)
        });
    let current = donor_source
        .lines
        .iter()
        .filter_map(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .filter(|deep| lm2_final_flow_line_ref(line, deep))
        })
        .min_by_key(|line| (line.page_index, line.line_index))?;
    Some((previous, current, terminal_callout_gap))
}

pub(super) fn lm2_source_ref_is_standalone_callout(line: &LiquidSourceLineRef) -> bool {
    let cleaned = strip_callout_sentinels_lm2(&line.text);
    cleaned
        .trim()
        .parse::<u16>()
        .is_ok_and(|marker| marker <= LM2_MAX_NOTE_MARKER)
}

pub(super) fn lm2_final_flow_line_ref(
    line_ref: &LiquidSourceLineRef,
    line: &DeepLiquidSourceLine,
) -> bool {
    if lm2_final_hard_furniture_line(line)
        || line.below_footnote_divider
        || line.in_ruled_cell
        || line.ruled_row_membership_exact
        || line.page_table_column_like
        || line.page_object_overlaps_image_bbox
    {
        return false;
    }
    if line.in_footnote_zone
        && !lm2_source_backed_body_display_line(line)
        && !lm2_body_sized_source_flow_ref(line_ref, line)
    {
        return false;
    }
    matches!(
        line_ref.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
    ) || lm2_source_backed_body_display_line(line)
        || line.segment_block_shape.eq_ignore_ascii_case("mixed")
}

pub(super) fn lm2_body_sized_source_flow_ref(
    line_ref: &LiquidSourceLineRef,
    line: &DeepLiquidSourceLine,
) -> bool {
    matches!(
        line_ref.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
    ) && !lm2_final_hard_furniture_line(line)
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
        && line.font_ratio_doc >= 0.95
        && line.text.chars().any(char::is_alphabetic)
}

pub(super) fn lm2_proven_body_continuation(
    before: &str,
    after: &str,
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
    terminal_callout_gap: bool,
    first_body_line_by_page: &HashMap<usize, usize>,
) -> bool {
    if lm2_displayed_lead_label_boundary(previous, current)
        || lm2_cross_page_displayed_lead_label_boundary(previous, current)
        || lm2_small_font_to_body_cliff_boundary(previous, current)
    {
        return false;
    }
    let before_without_callouts = strip_callout_sentinels_lm2(before);
    let open = lm2_reflow_paragraph_is_visibly_open(&before_without_callouts);
    let discretionary_hyphen = before_without_callouts.trim_end().ends_with('-')
        && after
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_alphabetic);
    let continuation_start = lm2_reflow_starts_like_continuation(after);
    let leading_callout = leading_sentineled_marker(after).is_some();
    let page_width = previous.page_width.max(current.page_width).max(1.0);
    let signed_left_delta = (current.left - previous.left) / page_width;
    let current_visual_left = if current.first_visual_left > 0.0 {
        current.first_visual_left
    } else {
        current.left
    };
    let visual_signed_left_delta = (current_visual_left - previous.left) / page_width;
    let font_ratio = previous.font_height.max(current.font_height)
        / previous.font_height.min(current.font_height).max(1.0);
    let indented_after_colon =
        before.trim_end().ends_with(':') && (current.left - previous.left) / page_width > 0.02;
    let next_mixed_lettered_list_item = previous.page_index == current.page_index
        && previous.segment_block_id == current.segment_block_id
        && lm2_safe_mixed_segment_line(previous)
        && lm2_safe_mixed_segment_line(current)
        && lm2_lettered_list_item_start(&current.text)
        && signed_left_delta < 0.0;
    if next_mixed_lettered_list_item {
        return false;
    }
    if previous.page_index == current.page_index {
        let adjacent = current.line_index == previous.line_index + 1
            || terminal_callout_gap && current.line_index == previous.line_index + 2;
        if !adjacent {
            return false;
        }
        if same_visual_row_fragment(previous, current)
            || leading_callout && same_visual_row_leading_callout_fragment(previous, current)
            || terminal_callout_gap && same_visual_row_after_standalone_callout(previous, current)
        {
            return terminal_callout_gap
                || open
                || discretionary_hyphen
                || continuation_start
                || leading_callout;
        }
        let same_margin_after_inline_callout = terminal_callout_gap
            && current.line_index == previous.line_index + 2
            && signed_left_delta.abs() <= 0.012
            && visual_signed_left_delta <= PARAGRAPH_FIRST_LINE_INDENT
            && vertical_gap(previous, current) <= 0.006
            && font_ratio <= 1.18;
        if same_margin_after_inline_callout {
            return true;
        }
        // A first line may be indented by roughly four percent of page width
        // and then return to the body margin on its wrapped second line. That
        // negative (outdent) delta is flow, while an equally large positive
        // delta remains a new-paragraph/blockquote boundary.
        if signed_left_delta > 0.04
            || signed_left_delta < -0.06
            || font_ratio > 1.18
            || indented_after_colon
            || lm2_final_paragraph_boundary(previous, current) && !discretionary_hyphen
        {
            return false;
        }
        return if previous.segment_block_id == current.segment_block_id {
            open || discretionary_hyphen || continuation_start || leading_callout
        } else {
            discretionary_hyphen || continuation_start || leading_callout
        };
    }
    // Facing journal pages alternate their body margin.  A continuation can
    // therefore move right by about four percent of page width or left by as
    // much as seven percent even though it is the first body line on the next
    // page.  Outdents are especially safe; right shifts still require visibly
    // open prose so closed, first-line-indented paragraphs remain boundaries.
    let cross_page_margin_ok = (-0.08..=0.05).contains(&signed_left_delta);
    let mirrored_open_continuation = open && cross_page_margin_ok;
    if current.page_index != previous.page_index + 1
        || first_body_line_by_page.get(&current.page_index).copied() != Some(current.line_index)
        || !(open || discretionary_hyphen || continuation_start)
        || paragraph_boundary(previous, current)
            && !discretionary_hyphen
            && !mirrored_open_continuation
    {
        return false;
    }
    !indented_after_colon && cross_page_margin_ok && font_ratio <= 1.18
}

pub(super) fn lm2_standalone_callout_block(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed
        .parse::<u16>()
        .is_ok_and(|marker| marker <= LM2_MAX_NOTE_MARKER)
        || leading_callout_marker_end(trimmed).is_some_and(|end| end == trimmed.len())
}

pub(super) fn lm2_merge_block_sources(
    target: usize,
    donor: usize,
    sources: &mut [LiquidBlockSourceLines],
) {
    let donor_lines = sources
        .iter()
        .find(|source| source.block_index == donor)
        .map(|source| source.lines.clone())
        .unwrap_or_default();
    if let Some(target_source) = sources
        .iter_mut()
        .find(|source| source.block_index == target)
    {
        target_source.lines.extend(donor_lines);
        target_source
            .lines
            .sort_by_key(|line| (line.page_index, line.line_index));
    }
}

pub(super) fn lm2_remove_block(
    index: usize,
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) {
    blocks.remove(index);
    sources.retain(|source| source.block_index != index);
    for source in sources {
        if source.block_index > index {
            source.block_index -= 1;
        }
    }
}

pub(super) fn flush_action_neutral_blocksplit(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    refs: &mut Vec<LiquidSourceLineRef>,
    role: LiquidBlockRole,
) {
    if refs.is_empty() {
        return;
    }
    let block_index = blocks.len();
    let mut text = String::new();
    for line in refs.iter() {
        let cleaned = clean_lm2_line_text(&line.text);
        if !cleaned.is_empty() {
            append_line(&mut text, &cleaned);
        }
    }
    if text.trim().is_empty() {
        refs.clear();
        return;
    }
    blocks.push(LiquidBlock {
        role,
        text,
        label: None,
    });
    sources.push(LiquidBlockSourceLines {
        block_index,
        lines: std::mem::take(refs),
    });
}

/// Undo any broad block grouping that swallowed a later numbered/lettered
/// outline item into the preceding heading. Provenance lines are authoritative
/// here: a fresh marker after the first source line starts a fresh block.
pub(super) fn apply_heading_outline_splits(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if blocks.is_empty() || sources.is_empty() {
        return 0;
    }
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut split_count = 0usize;

    for (old_index, mut block) in old_blocks.into_iter().enumerate() {
        let refs = source_by_block.remove(&old_index).unwrap_or_default();
        if matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) && refs
            .first()
            .is_some_and(|source| lm2_lowercase_roman_heading_source(source, &line_by_id))
        {
            block.role = LiquidBlockRole::Heading;
        }
        if matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Marginalia
        ) && refs.len() == 1
            && let Some(role) = lm2_paragraph_outline_anchor(&refs[0], &line_by_id)
        {
            block.role = role;
            split_count += 1;
        }
        if block.role == LiquidBlockRole::Paragraph
            && let Some(groups) = lm2_paragraph_outline_groups(&refs, &line_by_id)
        {
            split_count += groups.len().saturating_sub(1);
            for (role, group) in groups {
                let mut text = String::new();
                for line in &group {
                    let cleaned = clean_lm2_line_text(&line.text);
                    if !cleaned.is_empty() {
                        append_line(&mut text, &cleaned);
                    }
                }
                if text.trim().is_empty() {
                    continue;
                }
                let block_index = rebuilt_blocks.len();
                rebuilt_blocks.push(LiquidBlock {
                    role,
                    text,
                    label: block.label.clone(),
                });
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: group,
                });
            }
            continue;
        }
        if !matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) || refs.len() < 2
        {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            if !refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: refs,
                });
            }
            continue;
        }

        let mut groups: Vec<Vec<LiquidSourceLineRef>> = Vec::new();
        for line in refs {
            if !groups.is_empty() && lm2_heading_starts_new_outline_item(&line.text) {
                groups.push(Vec::new());
                split_count += 1;
            } else if groups.is_empty() {
                groups.push(Vec::new());
            }
            groups.last_mut().expect("heading group exists").push(line);
        }
        for group in groups {
            let mut text = String::new();
            for line in &group {
                let cleaned = clean_lm2_line_text(&line.text);
                if !cleaned.is_empty() {
                    append_line(&mut text, &cleaned);
                }
            }
            if text.trim().is_empty() {
                continue;
            }
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: block.role,
                text,
                label: block.label.clone(),
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: group,
            });
        }
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    split_count
}

pub(super) fn lm2_paragraph_outline_groups(
    refs: &[LiquidSourceLineRef],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> Option<Vec<(LiquidBlockRole, Vec<LiquidSourceLineRef>)>> {
    if refs.len() < 2 {
        return None;
    }
    let mut groups = Vec::new();
    let mut cursor = 0usize;
    let mut found = false;

    while cursor < refs.len() {
        let anchor = (cursor..refs.len()).find(|index| {
            lm2_paragraph_outline_anchor(&refs[*index], line_by_id).is_some()
                || lm2_early_page_terse_numeric_outline(&refs[*index])
                || lm2_terse_lowercase_lettered_outline(&refs[*index])
        });
        let Some(anchor) = anchor else {
            if cursor < refs.len() {
                groups.push((LiquidBlockRole::Paragraph, refs[cursor..].to_vec()));
            }
            break;
        };
        found = true;
        if anchor > cursor {
            groups.push((LiquidBlockRole::Paragraph, refs[cursor..anchor].to_vec()));
        }
        let role = lm2_paragraph_outline_anchor(&refs[anchor], line_by_id)
            .or_else(|| {
                lm2_early_page_terse_numeric_outline(&refs[anchor])
                    .then_some(LiquidBlockRole::Subheading)
                    .or_else(|| {
                        lm2_terse_lowercase_lettered_outline(&refs[anchor])
                            .then_some(LiquidBlockRole::Subheading)
                    })
            })
            .expect("outline anchor role remains available");
        let mut end = anchor + 1;
        while end < refs.len()
            && end - anchor < 5
            && lm2_paragraph_outline_continuation(&refs[end - 1], &refs[end], line_by_id)
        {
            end += 1;
        }
        groups.push((role, refs[anchor..end].to_vec()));
        cursor = end;
    }

    found.then_some(groups)
}

/// Some journals place a terse run-in heading immediately below the running
/// head. A page-relative small-font prior can make that early line look
/// footnote-like, but the first dozen physical lines are not the footnote band.
pub(super) fn lm2_early_page_terse_numeric_outline(source: &LiquidSourceLineRef) -> bool {
    source.line_index <= 12
        && source.note_markers.is_empty()
        && lm2_terse_numeric_outline_text(&source.text)
}

pub(super) fn lm2_terse_lowercase_lettered_outline(source: &LiquidSourceLineRef) -> bool {
    if !source.note_markers.is_empty() {
        return false;
    }
    let stripped = lm2_strip_leading_star_pagination(&source.text);
    let trimmed = stripped.trim();
    let token = trimmed.split_whitespace().next().unwrap_or_default();
    let marker = token.trim_end_matches(['.', ')', ':']);
    marker.len() == 1
        && marker.chars().all(|ch| ch.is_ascii_lowercase())
        && marker.len() != token.len()
        && (3..=12).contains(&word_count(trimmed))
        && !trimmed
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':' | '?' | '!'))
        && lm2_title_case_numbered_outline(trimmed)
}

pub(super) fn lm2_terse_numeric_outline_text(text: &str) -> bool {
    let stripped = lm2_strip_leading_star_pagination(text);
    let trimmed = stripped.trim();
    let token = trimmed.split_whitespace().next().unwrap_or_default();
    let marker = token.trim_end_matches(['.', ')', ':']);
    let remainder = trimmed[token.len()..].trim_start();
    marker
        .parse::<u16>()
        .ok()
        .is_some_and(|number| (1..=20).contains(&number))
        && marker.len() != token.len()
        && (2..=8).contains(&word_count(trimmed))
        && !trimmed
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':' | '?' | '!'))
        // This narrow early-page fallback is only for terse standalone labels.
        // Reject a title followed by run-in prose even when the combined line
        // happens to fit the fallback's eight-word ceiling.
        && !remainder.contains(". ")
        && !remainder.contains(['—', '–'])
        && remainder
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
}

/// A numbered title immediately sandwiched between contiguous body lines is
/// an outline heading even when it sits just above the page's footnotes and
/// the note detector mistakes its enumerator for a note marker.
pub(super) fn apply_sandwiched_numbered_outline_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    if blocks.len() < 3 {
        return 0;
    }
    let source_by_block = sources
        .iter()
        .enumerate()
        .map(|(source_index, source)| (source.block_index, source_index))
        .collect::<BTreeMap<_, _>>();
    let mut repaired = 0usize;

    for index in 1..blocks.len() - 1 {
        if blocks[index].role != LiquidBlockRole::Marginalia
            || !matches!(
                blocks[index - 1].role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
            )
            || !matches!(
                blocks[index + 1].role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
            )
        {
            continue;
        }
        let Some(&previous_source_index) = source_by_block.get(&(index - 1)) else {
            continue;
        };
        let Some(&current_source_index) = source_by_block.get(&index) else {
            continue;
        };
        let Some(&next_source_index) = source_by_block.get(&(index + 1)) else {
            continue;
        };
        let previous = &sources[previous_source_index].lines;
        let current = &sources[current_source_index].lines;
        let next = &sources[next_source_index].lines;
        let (Some(previous), [current], Some(next)) =
            (previous.last(), current.as_slice(), next.first())
        else {
            continue;
        };
        let marker = current.note_markers.first().copied();
        let lexical_heading = lm2_outline_heading_role(&current.text)
            == Some(LiquidBlockRole::Subheading)
            && (3..=8).contains(&word_count(&current.text))
            && lm2_title_case_numbered_outline(&current.text);
        let contiguous_body = previous.page_index == current.page_index
            && current.page_index == next.page_index
            && previous.line_index + 1 == current.line_index
            && current.line_index + 1 == next.line_index;
        if marker.is_some_and(|number| (1..=20).contains(&number))
            && lexical_heading
            && contiguous_body
        {
            blocks[index].role = LiquidBlockRole::Subheading;
            let current = &mut sources[current_source_index].lines[0];
            current.role = LiquidBlockRole::Subheading;
            current.note_markers.clear();
            repaired += 1;
        }
    }
    repaired
}

pub(super) fn lm2_title_case_numbered_outline(text: &str) -> bool {
    let stripped = lm2_strip_leading_star_pagination(text);
    let mut words = stripped.split_whitespace();
    let _marker = words.next();
    words.all(|word| {
        let cleaned = word.trim_matches(|ch: char| !ch.is_alphabetic());
        cleaned.is_empty()
            || matches!(
                cleaned.to_ascii_lowercase().as_str(),
                "a" | "an" | "and" | "for" | "in" | "of" | "on" | "or" | "the" | "to"
            )
            || cleaned.chars().next().is_some_and(char::is_uppercase)
    })
}

pub(super) fn lm2_paragraph_outline_anchor(
    source: &LiquidSourceLineRef,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> Option<LiquidBlockRole> {
    let text = lm2_strip_leading_star_pagination(&source.text);
    let lowercase_roman = lm2_lowercase_roman_outline_marker(text);
    let lowercase_special = matches!(
        normalize_text(text).as_str(),
        "abstract"
            | "acknowledgments"
            | "appendix"
            | "conclusion"
            | "discussion"
            | "introduction"
            | "limitations"
            | "methodology"
            | "methods"
            | "recommendations"
            | "references"
            | "results"
    );
    let mut role = lm2_outline_heading_role(text)
        .or_else(|| lowercase_roman.map(|_| LiquidBlockRole::Heading))
        .or_else(|| lowercase_special.then_some(LiquidBlockRole::Heading))?;
    let lower = text.to_ascii_lowercase();
    if text.contains(".-")
        || text.contains(".—")
        || text.contains('—')
        || text.contains("....")
        || !source.note_markers.is_empty()
        || word_count(text) > 20
        || lower.starts_with("part ")
        || lower.starts_with("chapter ")
    {
        return None;
    }
    let line = source
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())?;
    if lowercase_roman.is_some() {
        if !lm2_lowercase_roman_heading_source(source, line_by_id) {
            return None;
        }
        role = LiquidBlockRole::Heading;
    }
    if lowercase_special
        && !(line.bold
            && (line.centered
                || line.margin_centered
                || line.segment_block_shape.eq_ignore_ascii_case("heading")))
    {
        return None;
    }
    if lm2_low_footnote_zone_evidence(line) {
        return None;
    }
    if line.segment_block_toc_like
        || line.prev_line_has_dotleader
        || line.prev4_dotleader_count > 0
        || line.prev4_spaced_dotleader_count > 0
        || line.prev4_strong_dotleader_count > 0
        || line.prev4_toc_leader_context
    {
        return None;
    }
    let page_width = line.page_width.max(1.0);
    let width_ratio = (line.right - line.left).abs() / page_width;
    let strong_lexical_cue = lower.contains("question no.")
        || lower.contains("exception")
        || lower.contains("cancellation")
        || role == LiquidBlockRole::Heading;
    let marker = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_start_matches('(')
        .trim_end_matches(['.', ')', ':']);
    let small_numeric_marker = marker
        .parse::<u16>()
        .ok()
        .is_some_and(|number| (1..=20).contains(&number));
    let lettered_candidate = marker.len() == 1
        && marker.chars().all(|ch| ch.is_ascii_alphabetic())
        && !matches!(marker, "I" | "V" | "X");
    if marker.chars().all(|ch| ch.is_ascii_digit()) && !small_numeric_marker {
        return None;
    }
    let terse_numeric_heading = small_numeric_marker
        && (2..=8).contains(&word_count(text))
        && !text
            .trim_end()
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':'));
    let plausible_layout = if lettered_candidate {
        strong_lexical_cue || (width_ratio <= 0.60 && !text.contains(','))
    } else {
        strong_lexical_cue || terse_numeric_heading || width_ratio <= 0.60
    };
    plausible_layout.then_some(role)
}

pub(super) fn lm2_lowercase_roman_heading_source(
    source: &LiquidSourceLineRef,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    if lm2_lowercase_roman_outline_marker(&source.text).is_none() || !source.note_markers.is_empty()
    {
        return false;
    }
    let Some(line) = source
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())
    else {
        return false;
    };
    line.segment_block_shape == "heading"
        && line.bold
        && (line.centered || line.margin_centered)
        && !line.segment_block_toc_like
        && !line.segment_block_table_like
        && !line.page_table_column_like
        && !line.in_ruled_cell
        && !line.page_object_ruled_row_membership
        && !line.page_object_overlaps_image_bbox
        && !line.in_footnote_zone
        && !line.below_footnote_divider
}

pub(super) fn lm2_lowercase_roman_outline_marker(text: &str) -> Option<u16> {
    let token = text.trim_start().split_whitespace().next()?;
    if !token.ends_with('.') && !token.ends_with(')') {
        return None;
    }
    let marker = token.trim_matches(|ch: char| matches!(ch, '(' | ')' | '.'));
    if marker.is_empty()
        || marker.len() > 8
        || !marker.chars().all(|ch| {
            ch.is_ascii_lowercase() && matches!(ch, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')
        })
    {
        return None;
    }
    lm2_roman_marker_value(marker)
}

pub(super) fn lm2_roman_marker_value(marker: &str) -> Option<u16> {
    let mut total = 0u16;
    let mut previous = 0u16;
    for ch in marker.chars().rev() {
        let current = match ch.to_ascii_lowercase() {
            'i' => 1,
            'v' => 5,
            'x' => 10,
            'l' => 50,
            'c' => 100,
            'd' => 500,
            'm' => 1000,
            _ => return None,
        };
        if current < previous {
            total = total.checked_sub(current)?;
        } else {
            total = total.checked_add(current)?;
            previous = current;
        }
    }
    (total > 0).then_some(total)
}

pub(super) fn lm2_paragraph_outline_continuation(
    previous: &LiquidSourceLineRef,
    next: &LiquidSourceLineRef,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    if lm2_outline_heading_role(&next.text).is_some()
        || !next
            .text
            .trim_start()
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase)
    {
        return false;
    }
    let Some(previous_line) = previous
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())
    else {
        return false;
    };
    let Some(next_line) = next
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())
    else {
        return false;
    };
    if previous_line.page_index != next_line.page_index
        || next_line.line_index != previous_line.line_index + 1
    {
        return false;
    }
    let max_height = (previous_line.top - previous_line.bottom)
        .abs()
        .max((next_line.top - next_line.bottom).abs())
        .max(0.1);
    (previous_line.font_height - next_line.font_height).abs() <= max_height * 0.22
        && lm2_vertical_interval_gap(previous_line, next_line) <= max_height * 0.90 + 2.0
}

/// Recombine physical lines that form one logical heading. Native model
/// emissions intentionally keep heading lines separate, so this pass uses the
/// source geometry and the document's outline grammar instead of relying on a
/// particular classifier runtime.
pub(super) fn apply_heading_continuation_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if blocks.len() < 2 || sources.is_empty() {
        return 0;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut merged_count = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        let mut merged = old_blocks[index].clone();
        let mut merged_refs = source_by_block.remove(&index).unwrap_or_default();
        let mut next = index + 1;
        while next < old_blocks.len() {
            let next_refs = source_by_block.get(&next).cloned().unwrap_or_default();
            let demote_to_prose = lm2_should_demote_heading_into_prose(
                &merged,
                &old_blocks[next],
                &merged_refs,
                &next_refs,
                &line_by_id,
            );
            if !demote_to_prose
                && !lm2_should_merge_heading_continuation(
                    &merged,
                    &old_blocks[next],
                    &merged_refs,
                    &next_refs,
                    &line_by_id,
                )
            {
                break;
            }
            append_line(&mut merged.text, &old_blocks[next].text);
            merged_refs.extend(source_by_block.remove(&next).unwrap_or_default());
            if demote_to_prose {
                merged.role = LiquidBlockRole::Paragraph;
            } else if merged.role == LiquidBlockRole::Paragraph {
                merged.role =
                    lm2_outline_heading_role(&merged.text).unwrap_or(LiquidBlockRole::Subheading);
            }
            merged_count += 1;
            next += 1;
        }

        let block_index = rebuilt_blocks.len();
        rebuilt_blocks.push(merged);
        if !merged_refs.is_empty() {
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: merged_refs,
            });
        }
        index = next;
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    merged_count
}

/// A wrapped outline heading can contribute only its first physical row to a
/// Heading block while PyMuPDF groups the short final row with the following
/// body paragraph. Move that one source row into the heading when the first
/// line ends open (typically a comma), the continuation is short and aligned,
/// and the next source row begins a new body segment.
pub(super) fn apply_heading_leading_source_continuation_split(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_position = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for block_index in 0..blocks.len().saturating_sub(1) {
        if !matches!(
            blocks[block_index].role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) || blocks[block_index + 1].role != LiquidBlockRole::Paragraph
            || !lm2_heading_starts_new_outline_item(&blocks[block_index].text)
            || !blocks[block_index]
                .text
                .trim_end()
                .ends_with([',', ':', '-', '\u{2013}', '\u{2014}'])
        {
            continue;
        }
        let (Some(current_position), Some(next_position)) = (
            source_position.get(&block_index).copied(),
            source_position.get(&(block_index + 1)).copied(),
        ) else {
            continue;
        };
        if sources[next_position].lines.len() < 2 {
            continue;
        }
        let (Some(previous_ref), Some(continuation_ref), Some(body_ref)) = (
            sources[current_position].lines.last(),
            sources[next_position].lines.first(),
            sources[next_position].lines.get(1),
        ) else {
            continue;
        };
        let (Some(previous), Some(continuation), Some(body)) = (
            previous_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            continuation_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            body_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
        ) else {
            continue;
        };
        let continuation_text = clean_lm2_line_text(&continuation_ref.text);
        if previous.page_index != continuation.page_index
            || previous.line_index.checked_add(1) != Some(continuation.line_index)
            || continuation.page_index != body.page_index
            || continuation.line_index.checked_add(1) != Some(body.line_index)
            || !(1..=12).contains(&word_count(&continuation_text))
            || continuation_text.trim_end().ends_with([':', ';', '?', '!'])
            || !body
                .text
                .trim_start()
                .chars()
                .find(|ch| ch.is_alphabetic())
                .is_some_and(char::is_uppercase)
            || continuation.segment_block_id == body.segment_block_id
        {
            continue;
        }
        let page_width = previous.page_width.max(continuation.page_width).max(1.0);
        let previous_center = (previous.left + previous.right) * 0.5;
        let continuation_center = (continuation.left + continuation.right) * 0.5;
        let aligned = (previous_center - continuation_center).abs() <= page_width * 0.04;
        let similar_font = (previous.font_ratio_doc - continuation.font_ratio_doc).abs() <= 0.12
            && (previous.font_height - continuation.font_height).abs()
                <= previous.font_height.max(continuation.font_height).max(0.1) * 0.18;
        if !aligned || !similar_font {
            continue;
        }
        let Some(remainder) = blocks[block_index + 1]
            .text
            .strip_prefix(&continuation_text)
            .map(str::trim_start)
            .filter(|remainder| !remainder.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };

        append_line(&mut blocks[block_index].text, &continuation_text);
        blocks[block_index + 1].text = remainder;
        let mut moved = sources[next_position].lines.remove(0);
        moved.role = blocks[block_index].role;
        moved.note_markers.clear();
        sources[current_position].lines.push(moved);
        repaired += 1;
    }
    repaired
}

pub(super) fn lm2_should_merge_heading_continuation(
    current: &LiquidBlock,
    next: &LiquidBlock,
    current_refs: &[LiquidSourceLineRef],
    next_refs: &[LiquidSourceLineRef],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let current_is_outline = lm2_heading_starts_new_outline_item(&current.text);
    let open_run_in_body = lm2_numbered_run_in_body(&current.text).is_some_and(|body| {
        !lm2_blocksplit_ends_like_paragraph(body)
            && next.role == LiquidBlockRole::Paragraph
            && lm2_reflow_starts_like_continuation(&next.text)
    });
    let current_is_heading = matches!(
        current.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    );
    let next_is_heading_candidate = matches!(
        next.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading | LiquidBlockRole::Paragraph
    );
    let char_limit = if current_is_outline { 360 } else { 200 };
    let word_limit = if current_is_outline { 55 } else { 30 };
    if (!current_is_heading && !current_is_outline)
        || !next_is_heading_candidate
        || (!open_run_in_body
            && (current.text.chars().count() + next.text.chars().count() > char_limit
                || word_count(&current.text) + word_count(&next.text) > word_limit))
        || lm2_heading_starts_new_outline_item(&next.text)
        || matches!(
            current.text.trim_end().chars().last(),
            Some('?') | Some('!')
        )
        || (!open_run_in_body && current.role != next.role && next_refs.len() != 1)
    {
        return false;
    }
    let Some(previous_ref) = current_refs.last() else {
        return false;
    };
    let Some(next_ref) = next_refs.first() else {
        return false;
    };
    if previous_ref.page_index != next_ref.page_index
        || next_ref.line_index <= previous_ref.line_index
        || next_ref.line_index - previous_ref.line_index > 2
    {
        return false;
    }
    let previous_line = previous_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied());
    let next_line = next_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied());
    let (Some(previous_line), Some(next_line)) = (previous_line, next_line) else {
        return false;
    };

    let page_width = previous_line.page_width.max(next_line.page_width).max(1.0);
    let previous_center = (previous_line.left + previous_line.right) * 0.5;
    let next_center = (next_line.left + next_line.right) * 0.5;
    let centered_alignment = (previous_line.centered
        || previous_line.margin_centered
        || next_line.centered
        || next_line.margin_centered)
        && (previous_center - next_center).abs() <= page_width * 0.06;
    let aligned =
        centered_alignment || (previous_line.left - next_line.left).abs() <= page_width * 0.06;
    if !aligned {
        return false;
    }

    let previous_height = (previous_line.top - previous_line.bottom).abs().max(0.1);
    let next_height = (next_line.top - next_line.bottom).abs().max(0.1);
    let max_height = previous_height.max(next_height);
    if (previous_line.font_height - next_line.font_height).abs() > max_height * 0.22
        || (previous_line.font_ratio_doc - next_line.font_ratio_doc).abs() > 0.20
        || lm2_vertical_interval_gap(previous_line, next_line) > max_height * 0.90 + 2.0
    {
        return false;
    }

    let first_has_marker = lm2_heading_starts_new_outline_item(&current.text);
    let current_trimmed = current.text.trim_end();
    let first_opens_continuation = current_trimmed.ends_with(':') || current_trimmed.ends_with('-');
    let both_all_caps =
        uppercase_ratio(&current.text) >= 0.85 && uppercase_ratio(&next.text) >= 0.85;
    first_has_marker || first_opens_continuation || both_all_caps || open_run_in_body
}

/// Body portion of an Arabic run-in heading (`1. Label. — Body`).  The
/// assembler uses this only to preserve body flow; Markdown remains
/// responsible for rendering the heading/body boundary.
pub(super) fn lm2_numbered_run_in_body(text: &str) -> Option<&str> {
    let trimmed = lm2_strip_leading_star_pagination(text);
    let marker = trimmed.split_whitespace().next()?;
    let digits = marker.strip_suffix('.')?;
    if digits.is_empty() || digits.len() > 2 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let after_marker = trimmed[marker.len()..].trim_start();
    let heading_end = after_marker.find('.')?;
    let mut body = after_marker[heading_end + 1..].trim_start();
    body = body.trim_start_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{FFFD}'
            )
    });
    (!body.is_empty()
        && body
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase))
    .then_some(body)
}

pub(super) fn lm2_should_demote_heading_into_prose(
    current: &LiquidBlock,
    next: &LiquidBlock,
    current_refs: &[LiquidSourceLineRef],
    next_refs: &[LiquidSourceLineRef],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    if !matches!(
        current.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) || next.role != LiquidBlockRole::Paragraph
        || current_refs.len() != 1
        || next_refs.is_empty()
        || lm2_heading_starts_new_outline_item(&current.text)
        || current.text.chars().count() > 120
        || !current.text.contains(". ")
        || current.text.trim_end().ends_with(['.', '?', '!', ':'])
        || !next
            .text
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
    {
        return false;
    }
    let Some(first) = current_refs[0]
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())
    else {
        return false;
    };
    let Some(second) = next_refs[0]
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id).copied())
    else {
        return false;
    };
    if first.page_index != second.page_index
        || second.line_index != first.line_index + 1
        || first.segment_block_id != second.segment_block_id
    {
        return false;
    }
    let first_height = (first.top - first.bottom).abs().max(0.1);
    let second_height = (second.top - second.bottom).abs().max(0.1);
    (first.font_height - second.font_height).abs() <= first_height.max(second_height) * 0.22
}

pub(super) fn lm2_vertical_interval_gap(
    first: &DeepLiquidSourceLine,
    second: &DeepLiquidSourceLine,
) -> f32 {
    let first_low = first.bottom.min(first.top);
    let first_high = first.bottom.max(first.top);
    let second_low = second.bottom.min(second.top);
    let second_high = second.bottom.max(second.top);
    if first_high < second_low {
        second_low - first_high
    } else if second_high < first_low {
        first_low - second_high
    } else {
        0.0
    }
}

pub(super) fn lm2_heading_starts_new_outline_item(text: &str) -> bool {
    lm2_outline_heading_role(text).is_some()
}

pub(super) fn lm2_outline_heading_role(text: &str) -> Option<LiquidBlockRole> {
    let trimmed = lm2_strip_leading_star_pagination(text);
    let lower = trimmed.to_ascii_lowercase();
    let canonical = lower
        .trim_matches(|ch: char| !ch.is_alphanumeric() && !ch.is_whitespace())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(
        canonical.as_str(),
        "abstract"
            | "acknowledgments"
            | "appendix"
            | "background"
            | "conclusion"
            | "discussion"
            | "executive summary"
            | "findings"
            | "introduction"
            | "limitations"
            | "literature review"
            | "methodology"
            | "methods"
            | "recommendations"
            | "references"
            | "related work"
            | "results"
    ) {
        return trimmed
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
            .then_some(LiquidBlockRole::Heading);
    }
    if lower.starts_with("part ") || lower.starts_with("chapter ") {
        return Some(LiquidBlockRole::Heading);
    }
    let token = trimmed.split_whitespace().next()?;
    let unwrapped = token.trim_start_matches('(');
    let marker = unwrapped.trim_end_matches(['.', ')', ':']);
    if marker.is_empty() || marker.len() == unwrapped.len() {
        return None;
    }
    if marker.chars().all(|ch| ch.is_ascii_digit()) {
        let remainder = trimmed[token.len()..].trim_start();
        if remainder
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase)
        {
            return None;
        }
        return Some(LiquidBlockRole::Subheading);
    }
    if (marker.len() > 1 || matches!(marker, "I" | "V" | "X"))
        && marker
            .chars()
            .all(|ch| matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
    {
        return Some(LiquidBlockRole::Heading);
    }
    if marker.len() == 1 && marker.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Some(LiquidBlockRole::Subheading);
    }
    None
}

pub(super) fn lm2_strip_leading_star_pagination(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(after_star) = trimmed.strip_prefix('*') else {
        return trimmed;
    };
    let digits = after_star
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if !(2..=4).contains(&digits) {
        return trimmed;
    }
    let rest = &after_star[digits..];
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return trimmed;
    }
    rest.trim_start()
}

pub(super) fn apply_deferred_marginalia_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    let mut total = 0usize;
    for _ in 0..8 {
        let reflowed = apply_deferred_marginalia_reflow_once(blocks, sources);
        if reflowed == 0 {
            break;
        }
        total += reflowed;
    }
    total
}

/// Preserve a front-matter abstract that continues onto the next page and
/// then transitions into an explicit `AUTHOR.` paragraph.  The continuation's
/// small type can otherwise look like footnote marginalia and wind up in the
/// generic Notes appendix.
pub(super) fn apply_front_matter_abstract_author_reflow(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    let source_position = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for block_index in 1..blocks.len() {
        if blocks[block_index].role != LiquidBlockRole::Marginalia {
            continue;
        }
        let Some(current_position) = source_position.get(&block_index).copied() else {
            continue;
        };
        if sources[current_position].lines.is_empty()
            || sources[current_position]
                .lines
                .iter()
                .any(|line| line.page_index > 2 || !line.note_markers.is_empty())
        {
            continue;
        }
        let first_page_doi_author_material =
            sources[current_position].lines.first().is_some_and(|line| {
                line.page_index == 0 && normalize_text(&line.text).starts_with("doi:")
            }) && sources[current_position].lines.iter().skip(1).any(|line| {
                line.text
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(|ch| matches!(ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'))
            });
        if first_page_doi_author_material {
            sources[current_position].lines.remove(0);
            let mut text = String::new();
            for line in &mut sources[current_position].lines {
                line.role = LiquidBlockRole::AuthorInfo;
                line.note_markers.clear();
                append_line(&mut text, &clean_lm2_line_text(&line.text));
            }
            blocks[block_index].text = collapse_whitespace(&text).trim().to_owned();
            blocks[block_index].role = LiquidBlockRole::AuthorInfo;
            blocks[block_index].label = None;
            repaired += 1;
            continue;
        }
        let Some(author_line_position) = sources[current_position]
            .lines
            .iter()
            .position(|line| normalize_text(&line.text).starts_with("author."))
        else {
            continue;
        };
        if author_line_position == 0 {
            blocks[block_index].role = LiquidBlockRole::AuthorInfo;
            repaired += 1;
            continue;
        }
        let Some(abstract_index) = (0..block_index)
            .rev()
            .find(|index| blocks[*index].role == LiquidBlockRole::Abstract)
        else {
            continue;
        };
        let Some(abstract_position) = source_position.get(&abstract_index).copied() else {
            continue;
        };
        let author_offset = blocks[block_index]
            .text
            .to_ascii_lowercase()
            .find("author.")
            .unwrap_or(0);
        if author_offset == 0 {
            continue;
        }
        let continuation = blocks[block_index].text[..author_offset].trim().to_owned();
        let author = blocks[block_index].text[author_offset..].trim().to_owned();
        if continuation.is_empty() || author.is_empty() {
            continue;
        }
        append_line(&mut blocks[abstract_index].text, &continuation);
        blocks[block_index].text = author;
        blocks[block_index].role = LiquidBlockRole::AuthorInfo;
        blocks[block_index].label = None;

        let mut author_lines = sources[current_position]
            .lines
            .split_off(author_line_position);
        for line in &mut sources[current_position].lines {
            line.role = LiquidBlockRole::Abstract;
            line.note_markers.clear();
        }
        let mut continuation_lines = std::mem::take(&mut sources[current_position].lines);
        sources[abstract_position]
            .lines
            .append(&mut continuation_lines);
        for line in &mut author_lines {
            line.role = LiquidBlockRole::AuthorInfo;
            line.note_markers.clear();
        }
        sources[current_position].lines = author_lines;
        repaired += 1;
    }
    repaired
}

/// Split a first-page byline from an immediately following unlabelled abstract
/// when the PDF text layer flattened both into one Paragraph block.  A short
/// starred person name immediately after an explicit Title is authoritative;
/// ordinary paragraph geometry is not, because display bylines and the first
/// abstract line are often left-aligned identically.  Continue the Abstract
/// role across page furniture, but never beyond a contents block, page two, or
/// the first substantive section heading.
pub(super) fn apply_front_matter_byline_abstract_split(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let candidate_index = blocks.iter().enumerate().find_map(|(index, block)| {
        if block.role != LiquidBlockRole::Paragraph
            || !blocks[..index]
                .iter()
                .any(|previous| previous.role == LiquidBlockRole::Title)
            || blocks[..index].iter().any(|previous| {
                matches!(
                    previous.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                ) && lm2_substantive_section_heading(&previous.text)
            })
        {
            return None;
        }
        let refs = source_by_block.get(&index)?;
        let [first, second, ..] = refs.as_slice() else {
            return None;
        };
        let byline = clean_lm2_line_text(&first.text);
        let opening = clean_lm2_line_text(&second.text);
        (first.page_index <= 1
            && first.page_index == second.page_index
            && first.line_index.checked_add(1) == Some(second.line_index)
            && lm2_front_matter_starred_byline(&byline)
            && word_count(&opening) >= 6
            && !looks_like_lm2_author_heading(&opening)
            && block
                .text
                .strip_prefix(&byline)
                .is_some_and(|remainder| !remainder.trim().is_empty()))
        .then_some(index)
    });
    let Some(candidate_index) = candidate_index else {
        return 0;
    };

    let mut source_by_block = std::mem::take(sources)
        .into_iter()
        .map(|source| (source.block_index, source.lines))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len() + 1);
    let mut rebuilt_sources = Vec::with_capacity(source_by_block.len() + 1);
    let mut abstract_mode = false;
    let mut repaired = 0usize;

    for (old_index, mut block) in old_blocks.into_iter().enumerate() {
        let mut refs = source_by_block.remove(&old_index).unwrap_or_default();
        if old_index == candidate_index {
            let mut abstract_refs = refs.split_off(1);
            let mut byline_ref = refs.pop().expect("candidate has a byline source line");
            let byline = clean_lm2_line_text(&byline_ref.text);
            let abstract_text = block
                .text
                .strip_prefix(&byline)
                .expect("candidate text starts with its byline")
                .trim()
                .to_owned();
            byline_ref.role = LiquidBlockRole::AuthorInfo;
            for line in &mut abstract_refs {
                line.role = LiquidBlockRole::Abstract;
                line.note_markers.clear();
            }

            let byline_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: LiquidBlockRole::AuthorInfo,
                text: byline,
                label: None,
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index: byline_index,
                lines: vec![byline_ref],
            });

            let abstract_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: LiquidBlockRole::Abstract,
                text: abstract_text,
                label: None,
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index: abstract_index,
                lines: abstract_refs,
            });
            abstract_mode = true;
            repaired += 1;
            continue;
        }

        if abstract_mode {
            let first_page = refs.first().map(|line| line.page_index);
            if lm2_grouped_contents_block(&block.text)
                || matches!(
                    block.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                ) && lm2_substantive_section_heading(&block.text)
                || first_page.is_some_and(|page| page > 1)
            {
                abstract_mode = false;
            } else if block.role == LiquidBlockRole::Paragraph
                && first_page.is_some_and(|page| page <= 1)
            {
                block.role = LiquidBlockRole::Abstract;
                for line in &mut refs {
                    line.role = LiquidBlockRole::Abstract;
                    line.note_markers.clear();
                }
                repaired += 1;
            }
        }

        let block_index = rebuilt_blocks.len();
        rebuilt_blocks.push(block);
        if !refs.is_empty() {
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
        }
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_front_matter_starred_byline(text: &str) -> bool {
    let text = text.trim();
    let words = word_count(text);
    let has_trailing_marker = text
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'));
    (2..=8).contains(&words)
        && has_trailing_marker
        && title_case_ratio(text) >= 0.55
        && !text.contains([':', '?', '!'])
}

/// A contents page can lose its dot leaders in the PDF text layer.  Once the
/// page is grouped, its dense run of page locators and outline labels is much
/// stronger evidence than any one line-level role.  Keep it as explicit Noise
/// so neither fake numeric note heads nor rescued prose reach Markdown.
pub(super) fn apply_front_matter_contents_block_suppression(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    let mut repaired = 0usize;
    for source in sources.iter_mut() {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        let Some(page_index) = source.lines.first().map(|line| line.page_index) else {
            continue;
        };
        if page_index > 2
            || source
                .lines
                .iter()
                .any(|line| line.page_index != page_index)
            || !lm2_grouped_contents_block(&block.text)
        {
            continue;
        }
        if block.role != LiquidBlockRole::Noise {
            repaired += 1;
        }
        block.role = LiquidBlockRole::Noise;
        block.label = None;
        for line in &mut source.lines {
            line.role = LiquidBlockRole::Noise;
            line.note_markers.clear();
        }
    }
    repaired
}

pub(super) fn lm2_grouped_contents_block(text: &str) -> bool {
    let normalized = collapse_whitespace(text);
    let lower = normalize_text(&normalized);
    let page_locators = normalized
        .split_whitespace()
        .filter_map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_ascii_digit())
                .parse::<u16>()
                .ok()
        })
        .filter(|value| (100..=9999).contains(value))
        .count();
    if page_locators < 5 {
        return false;
    }
    let outline_labels = normalized
        .split_whitespace()
        .filter(|token| {
            let token = token.trim_matches(|ch: char| matches!(ch, '(' | ')' | '[' | ']'));
            let marker = token.trim_end_matches(['.', ')', ':']);
            marker.len() != token.len()
                && (marker.chars().all(|ch| ch.is_ascii_digit())
                    || marker.len() == 1 && marker.chars().all(|ch| ch.is_ascii_alphabetic())
                    || marker.len() <= 6
                        && marker.chars().all(|ch| {
                            matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x' | 'l' | 'c')
                        }))
        })
        .count();
    lower.contains("article contents")
        || lower.contains("table of contents")
        || (outline_labels >= 3 && lower.contains("conclusion "))
}

/// Restore a single body/table row that grouping emitted after the Paragraph
/// which physically surrounds it.  The target must have an exact one-line
/// source gap, the displaced block must be the immediately following block and
/// contain exactly that row, and typography must prove body or ruled-cell
/// membership. This repairs callout-bearing lines without globally sorting
/// blocks or rebuilding text (which would discard recovered sentinels).
pub(super) fn apply_source_gap_body_line_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for _ in 0..16 {
        let definitions = lm2_explicit_definition_markers(sources, &line_by_id);
        let owner_by_coordinate = sources
            .iter()
            .enumerate()
            .flat_map(|(source_position, source)| {
                source
                    .lines
                    .iter()
                    .enumerate()
                    .map(move |(line_position, line)| {
                        (
                            (line.page_index, line.line_index),
                            (source_position, line_position, source.block_index),
                        )
                    })
            })
            .collect::<HashMap<_, _>>();
        let mut operation = None;

        'target: for (target_position, source) in sources.iter().enumerate() {
            let Some(target_block) = blocks.get(source.block_index) else {
                continue;
            };
            if target_block.role != LiquidBlockRole::Paragraph || source.lines.len() < 2 {
                continue;
            }
            for current_position in 1..source.lines.len() {
                let previous = &source.lines[current_position - 1];
                let current = &source.lines[current_position];
                if previous.page_index != current.page_index
                    || previous.line_index.checked_add(2) != Some(current.line_index)
                {
                    continue;
                }
                let missing_index = previous.line_index + 1;
                let Some((candidate_position, candidate_line_position, candidate_block_index)) =
                    owner_by_coordinate
                        .get(&(current.page_index, missing_index))
                        .copied()
                else {
                    continue;
                };
                if candidate_block_index != source.block_index + 1
                    || candidate_position <= target_position
                    || sources[candidate_position].lines.len() != 1
                    || candidate_line_position != 0
                {
                    continue;
                }
                let Some(candidate_block) = blocks.get(candidate_block_index) else {
                    continue;
                };
                if !matches!(
                    candidate_block.role,
                    LiquidBlockRole::Paragraph
                        | LiquidBlockRole::Marginalia
                        | LiquidBlockRole::Table
                ) {
                    continue;
                }
                let candidate_ref = &sources[candidate_position].lines[0];
                let Some(candidate_line) = candidate_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                else {
                    continue;
                };
                let table_evidence = candidate_line.in_ruled_cell
                    || candidate_line.ruled_row_membership_exact
                    || candidate_line.page_object_ruled_row_membership
                    || candidate_line.page_table_column_like;
                if !table_evidence && !lm2_body_sized_reflow_line(candidate_line) {
                    continue;
                }
                if !candidate_ref.note_markers.is_empty()
                    && !candidate_ref
                        .note_markers
                        .iter()
                        .all(|marker| definitions.contains(marker))
                {
                    continue;
                }
                let current_needle = clean_lm2_line_text(&current.text);
                let previous_needle = clean_lm2_line_text(&previous.text);
                let candidate_text = candidate_block.text.trim().to_owned();
                if current_needle.is_empty()
                    || previous_needle.is_empty()
                    || candidate_text.is_empty()
                {
                    continue;
                }
                let Some(previous_offset) = target_block.text.find(&previous_needle) else {
                    continue;
                };
                let search_from = previous_offset + previous_needle.len();
                let Some(relative_current) = target_block.text[search_from..].find(&current_needle)
                else {
                    continue;
                };
                let insertion_offset = search_from + relative_current;
                operation = Some((
                    target_position,
                    current_position,
                    candidate_position,
                    source.block_index,
                    candidate_block_index,
                    insertion_offset,
                    candidate_text,
                ));
                break 'target;
            }
        }

        let Some((
            target_position,
            current_position,
            candidate_position,
            target_block_index,
            candidate_block_index,
            insertion_offset,
            candidate_text,
        )) = operation
        else {
            break;
        };

        let mut inserted = candidate_text;
        if !inserted.chars().last().is_some_and(char::is_whitespace) {
            inserted.push(' ');
        }
        blocks[target_block_index]
            .text
            .insert_str(insertion_offset, &inserted);
        let mut candidate_ref = sources[candidate_position].lines[0].clone();
        candidate_ref.role = LiquidBlockRole::Paragraph;
        sources[target_position]
            .lines
            .insert(current_position, candidate_ref);

        blocks.remove(candidate_block_index);
        sources.remove(candidate_position);
        for source in sources.iter_mut() {
            if source.block_index > candidate_block_index {
                source.block_index -= 1;
            }
        }
        repaired += 1;
    }

    repaired
}

/// Restore a body-sized physical row that a late furniture guard isolated as
/// Noise between the paragraphs immediately above and below it.  The source
/// rows must be consecutive. Ordinarily the isolated row must retain a
/// Paragraph hint and share its extraction segment with one of its neighbors.
/// A second, equally narrow path accepts a body-sized false Noise row whose
/// terminal discretionary-hyphen artifact joins the next lowercase row at the
/// same body margin. This is the inverse of a false single-row furniture split,
/// not a general Noise recovery rule.
pub(super) fn apply_sandwiched_body_noise_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for block_index in 0..blocks.len().saturating_sub(2) {
            if blocks[block_index].role != LiquidBlockRole::Paragraph
                || blocks[block_index + 1].role != LiquidBlockRole::Noise
                || blocks[block_index + 2].role != LiquidBlockRole::Paragraph
            {
                continue;
            }
            let (Some(previous_position), Some(candidate_position), Some(next_position)) = (
                source_position.get(&block_index).copied(),
                source_position.get(&(block_index + 1)).copied(),
                source_position.get(&(block_index + 2)).copied(),
            ) else {
                continue;
            };
            if sources[candidate_position].lines.len() != 1 {
                continue;
            }
            let (Some(previous_ref), Some(candidate_ref), Some(next_ref)) = (
                sources[previous_position].lines.last(),
                sources[candidate_position].lines.first(),
                sources[next_position].lines.first(),
            ) else {
                continue;
            };
            let (Some(previous), Some(candidate), Some(next)) = (
                previous_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                candidate_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                next_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            let ordinary_paragraph_hint = candidate.role_hint == Some(LiquidBlockRole::Paragraph)
                && (candidate.segment_block_id == previous.segment_block_id
                    || candidate.segment_block_id == next.segment_block_id);
            let terminal_dehyphen_bridge = {
                let candidate_clean = clean_lm2_line_text(&candidate.text);
                let horizontal_tolerance =
                    (candidate.page_width.max(next.page_width) * 0.008).max(2.5);
                let candidate_center = (candidate.top + candidate.bottom) * 0.5;
                let next_center = (next.top + next.bottom) * 0.5;
                let row_step = (candidate_center - next_center).abs();
                lm2_reflow_paragraph_is_visibly_open(&blocks[block_index].text)
                    && lm2_reflow_starts_like_continuation(&candidate.text)
                    && (candidate.text.trim_end().ends_with('\u{0002}')
                        || candidate_clean.trim_end().ends_with('-'))
                    && lm2_reflow_starts_like_continuation(&next.text)
                    && (candidate.left - next.left).abs() <= horizontal_tolerance
                    && row_step > 0.0
                    && row_step <= candidate.font_height.max(next.font_height) * 1.8
                    && lm2_body_sized_reflow_line(previous)
                    && lm2_body_sized_reflow_line(next)
                    && !previous.doc_repeated_edge_text
                    && !next.doc_repeated_edge_text
            };
            if previous.page_index != candidate.page_index
                || candidate.page_index != next.page_index
                || previous.line_index.checked_add(1) != Some(candidate.line_index)
                || candidate.line_index.checked_add(1) != Some(next.line_index)
                || !ordinary_paragraph_hint && !terminal_dehyphen_bridge
                || !lm2_body_sized_reflow_line(candidate)
                || candidate.doc_repeated_edge_text
                || candidate.page_object_overlaps_image_bbox
                || !candidate_ref.note_markers.is_empty()
                || word_count(&candidate.text) < 5
            {
                continue;
            }
            operation = Some((
                block_index,
                previous_position,
                candidate_position,
                next_position,
            ));
            break;
        }

        let Some((block_index, previous_position, candidate_position, next_position)) = operation
        else {
            break;
        };
        let candidate_text = blocks[block_index + 1].text.clone();
        let next_text = blocks[block_index + 2].text.clone();
        append_line(&mut blocks[block_index].text, &candidate_text);
        append_line(&mut blocks[block_index].text, &next_text);

        let mut candidate_refs = sources[candidate_position].lines.clone();
        for line in &mut candidate_refs {
            line.role = LiquidBlockRole::Paragraph;
        }
        let next_refs = sources[next_position].lines.clone();
        sources[previous_position].lines.extend(candidate_refs);
        sources[previous_position].lines.extend(next_refs);

        blocks.remove(block_index + 2);
        blocks.remove(block_index + 1);
        let mut removed_positions = [candidate_position, next_position];
        removed_positions.sort_unstable_by(|left, right| right.cmp(left));
        for position in removed_positions {
            sources.remove(position);
        }
        for source in sources.iter_mut() {
            if source.block_index > block_index + 2 {
                source.block_index -= 2;
            }
        }
        repaired += 1;
    }

    repaired
}

/// A PyMuPDF block can straddle the visual boundary between body text and the
/// footnote band.  When that happens, one or more small-font continuation rows
/// are appended to a Paragraph even though their source role or their physical
/// neighbor is Marginalia. Split only a trailing small-font run, preserving its
/// source order as a standalone Marginalia block for ordinary note assembly.
pub(super) fn apply_embedded_small_font_note_continuation_split(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut split_by_block = BTreeMap::<usize, usize>::new();

    for source in sources.iter() {
        let Some(block) = blocks.get(source.block_index) else {
            continue;
        };
        if !matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        ) || source.lines.is_empty()
        {
            continue;
        }
        let mut suffix_start = source.lines.len();
        while suffix_start > 0 {
            let line = &source.lines[suffix_start - 1];
            let Some(deep) = line
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                break;
            };
            let small_votes = [
                deep.font_ratio_page_ref,
                deep.font_ratio_page,
                deep.font_ratio_doc,
            ]
            .into_iter()
            .filter(|ratio| *ratio <= 0.88)
            .count();
            if !line.note_markers.is_empty()
                || small_votes < 2
                || deep.in_ruled_cell
                || deep.ruled_row_membership_exact
                || deep.page_table_column_like
                || deep.page_object_overlaps_image_bbox
            {
                break;
            }
            let physical_note_evidence = deep.in_footnote_zone
                || deep.below_footnote_divider
                || deep.doc_footnote_continuation
                || deep.doc_footnote_state && deep.segment_block_footnote_like;
            if !physical_note_evidence
                || deep.segment_block_shape.eq_ignore_ascii_case("body")
                    && !deep.in_footnote_zone
                    && !deep.below_footnote_divider
            {
                break;
            }
            suffix_start -= 1;
        }
        if suffix_start < source.lines.len() {
            split_by_block.insert(source.block_index, suffix_start);
        }
    }
    if split_by_block.is_empty() {
        return 0;
    }

    let mut source_by_block = std::mem::take(sources)
        .into_iter()
        .map(|source| (source.block_index, source.lines))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len() + split_by_block.len());
    let mut rebuilt_sources = Vec::with_capacity(source_by_block.len() + split_by_block.len());
    let mut repaired = 0usize;

    for (old_index, block) in old_blocks.into_iter().enumerate() {
        let Some(mut refs) = source_by_block.remove(&old_index) else {
            rebuilt_blocks.push(block);
            continue;
        };
        let Some(suffix_start) = split_by_block.get(&old_index).copied() else {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        };
        let mut note_refs = refs.split_off(suffix_start);
        for line in &mut note_refs {
            line.role = LiquidBlockRole::Marginalia;
            line.note_markers.clear();
        }
        let needle = note_refs
            .first()
            .map(|line| clean_lm2_line_text(&line.text))
            .unwrap_or_default();
        let Some(note_start) = (!needle.is_empty())
            .then(|| block.text.rfind(&needle))
            .flatten()
        else {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block);
            refs.extend(note_refs);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
            continue;
        };
        let body_text = block.text[..note_start].trim().to_owned();
        let note_text = block.text[note_start..].trim().to_owned();
        if !body_text.is_empty() && !refs.is_empty() {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(LiquidBlock {
                role: block.role,
                text: body_text,
                label: block.label.clone(),
            });
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: refs,
            });
        }
        let note_index = rebuilt_blocks.len();
        rebuilt_blocks.push(LiquidBlock {
            role: LiquidBlockRole::Marginalia,
            text: note_text,
            label: None,
        });
        rebuilt_sources.push(LiquidBlockSourceLines {
            block_index: note_index,
            lines: note_refs,
        });
        repaired += 1;
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

/// Repair the native grouping shape in which a single small-font continuation
/// row is emitted immediately before its numbered note head in block order.
/// Source coordinates must prove the opposite order: head N, continuation,
/// markerless note text, then head N+1. The continuation must share the head's
/// physical segment, while the following row must already belong to a note
/// block. This is deliberately narrower than a general block sort.
pub(super) fn apply_inverted_numbered_note_continuation_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for _ in 0..16 {
        let owner_by_coordinate = sources
            .iter()
            .enumerate()
            .flat_map(|(source_position, source)| {
                source
                    .lines
                    .iter()
                    .enumerate()
                    .map(move |(line_position, line)| {
                        (
                            (line.page_index, line.line_index),
                            (source_position, line_position, source.block_index),
                        )
                    })
            })
            .collect::<HashMap<_, _>>();
        let mut operation = None;

        for (candidate_position, source) in sources.iter().enumerate() {
            let Some(block) = blocks.get(source.block_index) else {
                continue;
            };
            if !matches!(
                block.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || source.lines.len() != 1
            {
                continue;
            }
            let candidate_ref = &source.lines[0];
            if !candidate_ref.note_markers.is_empty() {
                continue;
            }
            let Some(candidate) = candidate_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let small_votes = [
                candidate.font_ratio_page_ref,
                candidate.font_ratio_page,
                candidate.font_ratio_doc,
            ]
            .into_iter()
            .filter(|ratio| *ratio <= 0.88)
            .count();
            if small_votes < 2
                || candidate.in_ruled_cell
                || candidate.ruled_row_membership_exact
                || candidate.page_table_column_like
                || candidate.page_object_overlaps_image_bbox
            {
                continue;
            }
            let (Some(previous_line_index), Some(next_line_index)) = (
                candidate.line_index.checked_sub(1),
                candidate.line_index.checked_add(1),
            ) else {
                continue;
            };
            let (
                Some((head_position, head_line_position, head_block_index)),
                Some((tail_position, tail_line_position, tail_block_index)),
            ) = (
                owner_by_coordinate
                    .get(&(candidate.page_index, previous_line_index))
                    .copied(),
                owner_by_coordinate
                    .get(&(candidate.page_index, next_line_index))
                    .copied(),
            )
            else {
                continue;
            };
            if source.block_index.checked_add(1) != Some(head_block_index)
                || head_block_index.checked_add(1) != Some(tail_block_index)
                || !matches!(
                    blocks.get(head_block_index).map(|block| block.role),
                    Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                )
                || !matches!(
                    blocks.get(tail_block_index).map(|block| block.role),
                    Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                )
            {
                continue;
            }
            let Some(head_source) = sources.get(head_position) else {
                continue;
            };
            let Some(head_ref) = head_source.lines.get(head_line_position) else {
                continue;
            };
            if head_source.lines.len() != 1 || head_ref.note_markers.len() != 1 {
                continue;
            }
            let marker = head_ref.note_markers[0];
            let Some(head) = head_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let head_small_votes = [
                head.font_ratio_page_ref,
                head.font_ratio_page,
                head.font_ratio_doc,
            ]
            .into_iter()
            .filter(|ratio| *ratio <= 0.88)
            .count();
            if head_small_votes < 2
                || head.page_index != candidate.page_index
                || head.segment_block_id != candidate.segment_block_id
                || head.in_ruled_cell
                || head.ruled_row_membership_exact
                || head.page_object_overlaps_image_bbox
            {
                continue;
            }
            let Some(tail_ref) = sources
                .get(tail_position)
                .and_then(|tail| tail.lines.get(tail_line_position))
            else {
                continue;
            };
            let Some(next_marker) = marker.checked_add(1) else {
                continue;
            };
            let direct_next_head = tail_ref.note_markers.as_slice() == [next_marker];
            if !direct_next_head && !tail_ref.note_markers.is_empty() {
                continue;
            }
            let Some(tail) = tail_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            if !tail.in_footnote_zone
                || tail.in_ruled_cell
                || tail.ruled_row_membership_exact
                || tail.page_table_column_like
                || tail.page_object_overlaps_image_bbox
            {
                continue;
            }
            if !direct_next_head {
                let Some(following_index) = next_line_index.checked_add(1) else {
                    continue;
                };
                let Some((following_position, following_line_position, following_block_index)) =
                    owner_by_coordinate
                        .get(&(candidate.page_index, following_index))
                        .copied()
                else {
                    continue;
                };
                if following_block_index != tail_block_index
                    || sources
                        .get(following_position)
                        .and_then(|following| following.lines.get(following_line_position))
                        .is_none_or(|following| following.note_markers.as_slice() != [next_marker])
                {
                    continue;
                }
            }
            let cleaned = clean_lm2_line_text(&candidate_ref.text);
            if cleaned.is_empty() || !block.text.trim().ends_with(&cleaned) {
                continue;
            }
            operation = Some((
                candidate_position,
                head_position,
                source.block_index,
                head_block_index,
                cleaned,
            ));
            break;
        }

        let Some((
            candidate_position,
            head_position,
            candidate_block_index,
            head_block_index,
            cleaned,
        )) = operation
        else {
            break;
        };
        let mut moved = sources[candidate_position].lines[0].clone();
        moved.role = LiquidBlockRole::Marginalia;
        moved.note_markers.clear();
        sources[head_position].lines.push(moved);
        append_line(&mut blocks[head_block_index].text, &cleaned);

        blocks.remove(candidate_block_index);
        sources.remove(candidate_position);
        for source in sources.iter_mut() {
            if source.block_index > candidate_block_index {
                source.block_index -= 1;
            }
        }
        repaired += 1;
    }

    repaired
}

/// Identify the narrow cross-page ownership shape for an unnumbered note
/// continuation: every row in one physical segment is footnote-shaped, the
/// exact next row starts note N+1, and the preceding page's highest numbered
/// note is N with visibly unfinished prose. The open-prose and lowercase
/// continuation gates prevent a bottom-page body segment from being consumed
/// merely because coarse page geometry calls it footnote-like.
pub(super) fn physical_open_note_continuation_blocks(
    blocks: &[LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashSet<usize> {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let owner_by_coordinate = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().map(move |line| {
                (
                    (line.page_index, line.line_index),
                    (source.block_index, line),
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let mut candidate_segments =
        BTreeMap::<(usize, usize), (BTreeSet<usize>, HashSet<usize>)>::new();

    for source in sources {
        if source.lines.is_empty()
            || source
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty())
        {
            continue;
        }
        let deep_lines = source
            .lines
            .iter()
            .filter_map(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
            })
            .collect::<Vec<_>>();
        if deep_lines.len() != source.lines.len()
            || deep_lines.iter().any(|line| {
                !line.in_footnote_zone
                    || !line.segment_block_footnote_like
                    || !line.segment_block_shape.eq_ignore_ascii_case("footnote")
                    || line.in_ruled_cell
                    || line.ruled_row_membership_exact
                        && !line.below_footnote_divider
                        && line.segment_block_line_index != 0
                    || line.page_table_column_like
                    || line.page_object_overlaps_image_bbox
            })
        {
            continue;
        }
        let page_index = source.lines[0].page_index;
        let segment_block_id = deep_lines[0].segment_block_id;
        if source
            .lines
            .iter()
            .any(|line| line.page_index != page_index)
            || segment_block_id == 0
            || deep_lines.iter().any(|line| {
                line.page_index != page_index || line.segment_block_id != segment_block_id
            })
        {
            continue;
        }
        let entry = candidate_segments
            .entry((page_index, segment_block_id))
            .or_default();
        entry
            .0
            .extend(source.lines.iter().map(|line| line.line_index));
        entry.1.insert(source.block_index);
    }

    let mut protected = HashSet::new();
    for ((page_index, _), (line_indexes, block_indexes)) in candidate_segments {
        let line_indexes = line_indexes.into_iter().collect::<Vec<_>>();
        if line_indexes.is_empty()
            || line_indexes
                .windows(2)
                .any(|pair| pair[0].checked_add(1) != Some(pair[1]))
        {
            continue;
        }
        let Some(first_line_index) = line_indexes.first().copied() else {
            continue;
        };
        let Some((first_block_index, first_ref)) = owner_by_coordinate
            .get(&(page_index, first_line_index))
            .copied()
        else {
            continue;
        };
        if !block_indexes.contains(&first_block_index) {
            continue;
        }
        let starts_lowercase = first_ref
            .text
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase);
        let Some(next_line_index) = line_indexes.last().and_then(|last| last.checked_add(1)) else {
            continue;
        };
        let Some((next_block_index, next_ref)) = owner_by_coordinate
            .get(&(page_index, next_line_index))
            .copied()
        else {
            continue;
        };
        if !matches!(
            blocks.get(next_block_index).map(|block| block.role),
            Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
        ) || next_ref.note_markers.len() != 1
        {
            continue;
        }
        let next_marker = next_ref.note_markers[0];
        let Some(previous_page) = page_index.checked_sub(1) else {
            continue;
        };
        let previous_note = sources
            .iter()
            .filter(|previous_source| {
                matches!(
                    blocks
                        .get(previous_source.block_index)
                        .map(|block| block.role),
                    Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                )
            })
            .flat_map(|previous_source| {
                previous_source.lines.iter().flat_map(move |line| {
                    if line.page_index != previous_page {
                        return Vec::new();
                    }
                    line.note_markers
                        .iter()
                        .copied()
                        .map(|marker| (marker, previous_source.block_index))
                        .collect::<Vec<_>>()
                })
            })
            .max_by_key(|(marker, _)| *marker);
        let Some((previous_marker, previous_block_index)) = previous_note else {
            continue;
        };
        if previous_marker.checked_add(1) != Some(next_marker) {
            continue;
        }
        let previous_visibly_open = blocks.get(previous_block_index).is_some_and(|block| {
            block
                .text
                .trim_end()
                .chars()
                .rev()
                .find(|ch| {
                    !matches!(
                        *ch,
                        '"' | '\'' | ')' | ']' | '}' | '\u{2019}' | '\u{201d}' | CALLOUT_END
                    )
                })
                .is_some_and(|ch| {
                    ch.is_alphanumeric()
                        || matches!(
                            ch,
                            ',' | ';' | ':' | '/' | '(' | '-' | '\u{2010}'..='\u{2015}'
                        )
                })
        });
        let mut segment_text = String::new();
        let all_refs_marginalia = line_indexes.iter().all(|line_index| {
            owner_by_coordinate
                .get(&(page_index, *line_index))
                .is_some_and(|(_, line)| {
                    matches!(
                        line.role,
                        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
                    ) && line.note_markers.is_empty()
                })
        });
        for line_index in &line_indexes {
            if let Some((_, line)) = owner_by_coordinate.get(&(page_index, *line_index)) {
                append_line(&mut segment_text, &line.text);
            }
        }
        let uppercase_citation_continuation = all_refs_marginalia
            && blocks.get(previous_block_index).is_some_and(|previous| {
                lm2_citation_suffix_cross_page_bridge(&previous.text, &segment_text)
                    || lm2_uppercase_note_continuation_cue(&segment_text)
            });
        if (!previous_visibly_open && !uppercase_citation_continuation)
            || (!starts_lowercase && !uppercase_citation_continuation)
        {
            continue;
        }
        protected.extend(block_indexes);
    }

    protected
}

pub(super) fn lm2_uppercase_note_continuation_cue(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.starts_with("see ")
        || lower.starts_with("see, ")
        || lower.starts_with("but see ")
        || lower.starts_with("cf. ")
        || lower.starts_with("compare ")
        || lower.starts_with("accord ")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains(" v. ")
        || lower.contains(" f.2d ")
        || lower.contains(" f.3d ")
        || lower.contains(" f.4th ")
        || lower.contains(" u.s. ")
        || lower.contains(" u.s.c. ")
        || lower.contains('§')
        || [
            "(jan.", "(feb.", "(mar.", "(apr.", "(may ", "(june ", "(july ", "(aug.", "(sept.",
            "(oct.", "(nov.", "(dec.",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

/// Preserve only a markerless segment with exact cross-page note ownership
/// after all body-role rescue and source-backed splitting has completed. The
/// Markdown note assembler remains responsible for attaching the continuation
/// to the numbered definition.
pub(super) fn apply_final_physical_footnote_segment_guard(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let repaired_blocks = physical_open_note_continuation_blocks(blocks, sources, decoded)
        .into_iter()
        .filter(|block_index| {
            matches!(
                blocks.get(*block_index).map(|block| block.role),
                Some(LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote)
            )
        })
        .collect::<HashSet<_>>();

    for block_index in &repaired_blocks {
        if let Some(block) = blocks.get_mut(*block_index) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
        }
    }
    repaired_blocks.len()
}

/// Consolidate a numbered note whose physical tail was split across a
/// markerless note block and an earlier body block. Some extractors emit the
/// final short row in a new body-shaped segment, causing block order to invert
/// even though source-row order remains unambiguous. Require a numbered note
/// immediately before the complete physical interval, markerless donor blocks
/// wholly contained in that interval, and footnote geometry through the
/// penultimate row. At most one short, lowercase, sentence-ending final spill
/// may leave the footnote segment.
pub(super) fn apply_gapped_numbered_note_tail_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for _ in 0..8 {
        let owner_by_coordinate = sources
            .iter()
            .enumerate()
            .flat_map(|(source_position, source)| {
                source
                    .lines
                    .iter()
                    .enumerate()
                    .map(move |(line_position, line)| {
                        (
                            (line.page_index, line.line_index),
                            (source_position, line_position, source.block_index),
                        )
                    })
            })
            .collect::<HashMap<_, _>>();
        let mut operation = None;

        'candidate: for (_candidate_position, candidate_source) in sources.iter().enumerate() {
            let Some(candidate_block) = blocks.get(candidate_source.block_index) else {
                continue;
            };
            if !matches!(
                candidate_block.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || candidate_source.lines.len() < 2
                || candidate_source
                    .lines
                    .iter()
                    .any(|line| !line.note_markers.is_empty())
            {
                continue;
            }
            let mut candidate_coordinates = candidate_source
                .lines
                .iter()
                .map(|line| (line.page_index, line.line_index))
                .collect::<Vec<_>>();
            candidate_coordinates.sort_unstable();
            candidate_coordinates.dedup();
            if candidate_coordinates.len() != candidate_source.lines.len()
                || candidate_coordinates
                    .windows(2)
                    .any(|pair| pair[0].0 != pair[1].0)
                || !candidate_coordinates
                    .windows(2)
                    .any(|pair| pair[0].1.saturating_add(1) < pair[1].1)
            {
                continue;
            }
            let page_index = candidate_coordinates[0].0;
            let first_line_index = candidate_coordinates[0].1;
            let last_line_index = candidate_coordinates.last().expect("candidate rows").1;
            let Some(previous_line_index) = first_line_index.checked_sub(1) else {
                continue;
            };
            let Some((target_position, target_line_position, target_block_index)) =
                owner_by_coordinate
                    .get(&(page_index, previous_line_index))
                    .copied()
            else {
                continue;
            };
            let Some(target_block) = blocks.get(target_block_index) else {
                continue;
            };
            let Some(target_source) = sources.get(target_position) else {
                continue;
            };
            let target_markers = target_source
                .lines
                .iter()
                .flat_map(|line| line.note_markers.iter().copied())
                .collect::<BTreeSet<_>>();
            if candidate_source.block_index >= target_block_index
                || !matches!(
                    target_block.role,
                    LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
                )
                || target_line_position + 1 != target_source.lines.len()
                || target_markers.len() != 1
            {
                continue;
            }
            let Some(previous_deep) = target_source.lines[target_line_position]
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            if !previous_deep.segment_block_footnote_like
                || !previous_deep
                    .segment_block_shape
                    .eq_ignore_ascii_case("footnote")
                || !previous_deep.in_footnote_zone
                || previous_deep.in_ruled_cell
                || previous_deep.ruled_row_membership_exact
                || previous_deep.page_table_column_like
                || previous_deep.page_object_overlaps_image_bbox
            {
                continue;
            }

            let mut interval = Vec::new();
            for line_index in first_line_index..=last_line_index {
                let Some(owner) = owner_by_coordinate.get(&(page_index, line_index)).copied()
                else {
                    continue 'candidate;
                };
                interval.push((line_index, owner));
            }
            let donor_blocks = interval
                .iter()
                .map(|(_, (_, _, block_index))| *block_index)
                .collect::<BTreeSet<_>>();
            if !donor_blocks.contains(&candidate_source.block_index)
                || donor_blocks.len() < 2
                || donor_blocks.contains(&target_block_index)
            {
                continue;
            }
            for donor_block_index in &donor_blocks {
                let Some(donor_source) = sources
                    .iter()
                    .find(|source| source.block_index == *donor_block_index)
                else {
                    continue 'candidate;
                };
                if donor_source.lines.iter().any(|line| {
                    line.page_index != page_index
                        || line.line_index < first_line_index
                        || line.line_index > last_line_index
                        || !line.note_markers.is_empty()
                }) {
                    continue 'candidate;
                }
                if *donor_block_index != candidate_source.block_index
                    && !matches!(
                        blocks.get(*donor_block_index).map(|block| block.role),
                        Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                    )
                {
                    continue 'candidate;
                }
            }

            let first_deep = interval[0].1.0;
            let Some(first_deep) = sources[first_deep].lines[interval[0].1.1]
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            for (_, (source_position, line_position, _)) in
                interval.iter().take(interval.len().saturating_sub(1))
            {
                let Some(deep) = sources[*source_position].lines[*line_position]
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                else {
                    continue 'candidate;
                };
                if deep.segment_block_id != previous_deep.segment_block_id
                    || !deep.segment_block_footnote_like
                    || !deep.segment_block_shape.eq_ignore_ascii_case("footnote")
                    || !deep.in_footnote_zone
                    || deep.in_ruled_cell
                    || deep.ruled_row_membership_exact
                    || deep.page_table_column_like
                    || deep.page_object_overlaps_image_bbox
                {
                    continue 'candidate;
                }
            }
            let (final_source_position, final_line_position, final_block_index) =
                interval.last().expect("interval row").1;
            let Some(final_deep) = sources[final_source_position].lines[final_line_position]
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let final_in_footnote_segment = final_deep.segment_block_id
                == previous_deep.segment_block_id
                && final_deep.segment_block_footnote_like
                && final_deep
                    .segment_block_shape
                    .eq_ignore_ascii_case("footnote")
                && final_deep.in_footnote_zone
                && !final_deep.in_ruled_cell
                && !final_deep.ruled_row_membership_exact
                && !final_deep.page_table_column_like
                && !final_deep.page_object_overlaps_image_bbox;
            if !final_in_footnote_segment {
                let horizontal_tolerance =
                    (first_deep.page_width.max(final_deep.page_width) * 0.008).max(2.5);
                if final_block_index != candidate_source.block_index
                    || !lm2_body_sized_reflow_line(final_deep)
                    || final_deep.doc_repeated_edge_text
                    || (final_deep.left - first_deep.left).abs() > horizontal_tolerance
                    || !lm2_reflow_starts_like_continuation(&final_deep.text)
                    || word_count(&final_deep.text) > 8
                    || !lm2_blocksplit_ends_like_paragraph(final_deep.text.trim_end())
                {
                    continue;
                }
            }

            let ordered_refs = interval
                .iter()
                .map(|(_, (source_position, line_position, _))| {
                    sources[*source_position].lines[*line_position].clone()
                })
                .collect::<Vec<_>>();
            operation = Some((
                target_position,
                target_block_index,
                donor_blocks,
                ordered_refs,
            ));
            break;
        }

        let Some((target_position, target_block_index, donor_blocks, mut ordered_refs)) = operation
        else {
            break;
        };
        for line in &mut ordered_refs {
            line.role = LiquidBlockRole::Marginalia;
            line.note_markers.clear();
            append_line(&mut blocks[target_block_index].text, &line.text);
        }
        sources[target_position].lines.extend(ordered_refs);

        let removed_blocks = donor_blocks.iter().copied().collect::<Vec<_>>();
        for block_index in removed_blocks.iter().rev() {
            blocks.remove(*block_index);
        }
        sources.retain(|source| !donor_blocks.contains(&source.block_index));
        for source in sources.iter_mut() {
            let removed_before = removed_blocks
                .iter()
                .filter(|block_index| **block_index < source.block_index)
                .count();
            source.block_index -= removed_before;
        }
        repaired += 1;
    }

    repaired
}

/// Restore a physical note continuation that a grouping pass appended to a
/// body block after skipping over intervening note heads.  The source-line gap
/// and the immediately preceding Marginalia line are the contract; no document
/// title or citation text is consulted.
pub(super) fn apply_interleaved_note_continuation_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = apply_gapped_numbered_note_tail_reflow(blocks, sources, decoded);

    for _ in 0..16 {
        let owner_by_coordinate = sources
            .iter()
            .enumerate()
            .flat_map(|(source_position, source)| {
                source
                    .lines
                    .iter()
                    .enumerate()
                    .map(move |(line_position, line)| {
                        (
                            (line.page_index, line.line_index),
                            (source_position, line_position, source.block_index),
                        )
                    })
            })
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        'source: for (source_position, source) in sources.iter().enumerate() {
            let Some(block) = blocks.get(source.block_index) else {
                continue;
            };
            if !matches!(
                block.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || source.lines.len() < 2
            {
                continue;
            }
            for line_position in 1..source.lines.len() {
                let previous = &source.lines[line_position - 1];
                let candidate = &source.lines[line_position];
                if candidate.page_index != previous.page_index
                    || candidate.line_index <= previous.line_index.saturating_add(1)
                    || line_position + 1 != source.lines.len()
                {
                    continue;
                }
                let Some(candidate_line) = candidate
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                else {
                    continue;
                };
                let Some(previous_coordinate) = candidate.line_index.checked_sub(1) else {
                    continue;
                };
                let Some((target_position, _, target_block_index)) = owner_by_coordinate
                    .get(&(candidate.page_index, previous_coordinate))
                    .copied()
                else {
                    continue;
                };
                let Some(target_block) = blocks.get(target_block_index) else {
                    continue;
                };
                let target_has_note_provenance =
                    sources.get(target_position).is_some_and(|target| {
                        target.lines.iter().any(|line| {
                            !line.note_markers.is_empty()
                                || line
                                    .id
                                    .as_deref()
                                    .and_then(|id| line_by_id.get(id).copied())
                                    .is_some_and(|deep| {
                                        deep.doc_note_marker > 0
                                            || leading_explicit_numbered_note_marker(&deep.text)
                                                .is_some()
                                            || note_head_marker(&deep.text).is_some()
                                    })
                        })
                    });
                if !matches!(
                    target_block.role,
                    LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
                ) || !target_has_note_provenance
                    || !candidate_line.in_footnote_zone
                    || candidate_line.in_ruled_cell
                {
                    continue;
                }
                let bodyish_note_continuation = [
                    candidate_line.font_ratio_page_ref,
                    candidate_line.font_ratio_page,
                    candidate_line.font_ratio_doc,
                ]
                .into_iter()
                .filter(|ratio| *ratio >= 0.92)
                .count()
                    >= 2;
                if !bodyish_note_continuation {
                    continue;
                }
                let cleaned = clean_lm2_line_text(&candidate.text);
                if cleaned.is_empty() || !block.text.trim_end().ends_with(&cleaned) {
                    continue;
                }
                operation = Some((
                    source_position,
                    line_position,
                    target_position,
                    source.block_index,
                    target_block_index,
                    cleaned,
                ));
                break 'source;
            }
        }
        let Some((
            source_position,
            line_position,
            target_position,
            source_block_index,
            target_block_index,
            cleaned,
        )) = operation
        else {
            break;
        };

        let mut moved = sources[source_position].lines.remove(line_position);
        moved.role = LiquidBlockRole::Marginalia;
        moved.note_markers.clear();
        sources[target_position].lines.push(moved);
        if let Some(prefix) = blocks[source_block_index]
            .text
            .trim_end()
            .strip_suffix(&cleaned)
        {
            blocks[source_block_index].text = prefix.trim_end().to_owned();
        }
        append_line(&mut blocks[target_block_index].text, &cleaned);
        repaired += 1;
    }

    // A table prior can still capture the remaining tail of the very same
    // note.  Restore only source-provenance Marginalia that is physically
    // contiguous with a preceding note and starts like continuation prose.
    let owner_by_coordinate = sources
        .iter()
        .flat_map(|source| {
            source
                .lines
                .iter()
                .map(move |line| ((line.page_index, line.line_index), source.block_index))
        })
        .collect::<HashMap<_, _>>();
    for source in sources.iter() {
        let Some(block) = blocks.get(source.block_index) else {
            continue;
        };
        if block.role != LiquidBlockRole::Table
            || source.lines.is_empty()
            || source.lines.iter().any(|line| {
                line.role != LiquidBlockRole::Marginalia || !line.note_markers.is_empty()
            })
            || leading_numeric_token_marker(&block.text).is_some()
        {
            continue;
        }
        let first = &source.lines[0];
        let Some(previous_line_index) = first.line_index.checked_sub(1) else {
            continue;
        };
        let Some(previous_block_index) = owner_by_coordinate
            .get(&(first.page_index, previous_line_index))
            .copied()
        else {
            continue;
        };
        if !matches!(
            blocks.get(previous_block_index).map(|block| block.role),
            Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
        ) || source.lines.iter().any(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_none_or(|line| !line.in_footnote_zone || line.in_ruled_cell)
        }) {
            continue;
        }
        if let Some(block) = blocks.get_mut(source.block_index) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
            repaired += 1;
        }
    }

    // A cross-page note can resume below the next page's body and immediately
    // before the following numbered note.  Table priors are especially prone
    // to capturing short, citation-dense carryovers such as a year followed by
    // docket information.  A Paragraph can also capture the middle of one
    // physical footnote segment when only its first row retained a Marginalia
    // hint. Restore either shape only when the next source row is exactly the
    // next numbered definition and the preceding page ended with its
    // predecessor. A final Paragraph can retain all-Marginalia source roles;
    // in that shape the complete physical footnote segment plus an open prior
    // note replaces the discretionary-hyphen requirement. Those provenance
    // checks keep real tables and body prose out.
    let mut paragraph_segment_repairs = HashSet::new();
    for source in sources.iter() {
        let Some(block) = blocks.get(source.block_index) else {
            continue;
        };
        if source.lines.is_empty() {
            continue;
        }
        let paragraph_segment = block.role == LiquidBlockRole::Paragraph
            && source.lines.iter().all(|line| {
                line.role == LiquidBlockRole::Paragraph && line.note_markers.is_empty()
            })
            && source.lines.first().is_some_and(|first| {
                let Some(first_deep) = first
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                else {
                    return false;
                };
                let same_physical_segment = source.lines.iter().all(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied())
                        .is_some_and(|deep| {
                            deep.page_index == first_deep.page_index
                                && deep.segment_block_id == first_deep.segment_block_id
                                && deep.segment_block_footnote_like
                                && deep.segment_block_shape.eq_ignore_ascii_case("footnote")
                                && deep.in_footnote_zone
                                && !deep.in_ruled_cell
                                && !deep.ruled_row_membership_exact
                                && !deep.page_table_column_like
                                && !deep.page_object_overlaps_image_bbox
                        })
                });
                let preceding_same_segment_marginalia = first
                    .line_index
                    .checked_sub(1)
                    .and_then(|line_index| {
                        owner_by_coordinate
                            .get(&(first.page_index, line_index))
                            .copied()
                    })
                    .and_then(|previous_block_index| {
                        let previous_source = sources
                            .iter()
                            .find(|candidate| candidate.block_index == previous_block_index)?;
                        let previous_ref = previous_source.lines.last()?;
                        let previous_deep = previous_ref
                            .id
                            .as_deref()
                            .and_then(|id| line_by_id.get(id).copied())?;
                        Some(
                            matches!(
                                blocks.get(previous_block_index).map(|block| block.role),
                                Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                            ) && previous_ref.note_markers.is_empty()
                                && previous_deep.page_index == first_deep.page_index
                                && previous_deep.segment_block_id == first_deep.segment_block_id
                                && previous_deep.segment_block_footnote_like
                                && previous_deep.in_footnote_zone,
                        )
                    })
                    .unwrap_or(false);
                same_physical_segment && preceding_same_segment_marginalia
            });
        let near_rule_paragraph_segment = block.role == LiquidBlockRole::Paragraph
            && lm2_near_rule_cross_page_note_segment(source, &line_by_id);
        let table_carryover = block.role == LiquidBlockRole::Table
            && source.lines.iter().all(|line| {
                line.role == LiquidBlockRole::Marginalia && line.note_markers.is_empty()
            });
        let source_marginalia_carryover = block.role == LiquidBlockRole::Paragraph
            && source.lines.iter().all(|line| {
                line.role == LiquidBlockRole::Marginalia && line.note_markers.is_empty()
            })
            && source.lines.first().is_some_and(|first| {
                let Some(first_deep) = first
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                else {
                    return false;
                };
                source.lines.iter().all(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied())
                        .is_some_and(|deep| {
                            deep.page_index == first_deep.page_index
                                && deep.segment_block_id == first_deep.segment_block_id
                                && deep.segment_block_footnote_like
                                && deep.segment_block_shape.eq_ignore_ascii_case("footnote")
                                && deep.in_footnote_zone
                                && !deep.in_ruled_cell
                                && !deep.ruled_row_membership_exact
                                && !deep.page_table_column_like
                                && !deep.page_object_overlaps_image_bbox
                        })
                })
            });
        if !paragraph_segment
            && !near_rule_paragraph_segment
            && !table_carryover
            && !source_marginalia_carryover
        {
            continue;
        }
        let page_index = source.lines[0].page_index;
        if source
            .lines
            .iter()
            .any(|line| line.page_index != page_index)
            || source.lines.iter().any(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                    .is_none_or(|line| {
                        !line.in_footnote_zone && !near_rule_paragraph_segment
                            || line.in_ruled_cell
                            || table_carryover
                                && [
                                    line.font_ratio_page_ref,
                                    line.font_ratio_page,
                                    line.font_ratio_doc,
                                ]
                                .into_iter()
                                .filter(|ratio| *ratio <= 0.90)
                                .count()
                                    < 2
                    })
            })
        {
            continue;
        }
        let Some(next_line_index) = source
            .lines
            .last()
            .and_then(|line| line.line_index.checked_add(1))
        else {
            continue;
        };
        let Some(next_block_index) = owner_by_coordinate
            .get(&(page_index, next_line_index))
            .copied()
        else {
            continue;
        };
        if !matches!(
            blocks.get(next_block_index).map(|block| block.role),
            Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
        ) {
            continue;
        }
        let next_markers = sources
            .iter()
            .find(|candidate| candidate.block_index == next_block_index)
            .into_iter()
            .flat_map(|candidate| &candidate.lines)
            .filter(|line| line.page_index == page_index && line.line_index == next_line_index)
            .flat_map(|line| line.note_markers.iter().copied())
            .collect::<BTreeSet<_>>();
        if next_markers.len() != 1 {
            continue;
        }
        let next_marker = *next_markers.iter().next().expect("one next marker");
        let previous_page_note = page_index.checked_sub(1).and_then(|previous_page| {
            sources
                .iter()
                .filter(|candidate| {
                    matches!(
                        blocks.get(candidate.block_index).map(|block| block.role),
                        Some(LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote)
                    )
                })
                .flat_map(|candidate| {
                    candidate
                        .lines
                        .iter()
                        .filter(move |line| line.page_index == previous_page)
                        .flat_map(move |line| {
                            line.note_markers
                                .iter()
                                .copied()
                                .map(move |marker| (marker, candidate.block_index))
                        })
                })
                .max_by_key(|(marker, _)| *marker)
        });
        if previous_page_note.and_then(|(marker, _)| marker.checked_add(1)) != Some(next_marker) {
            continue;
        }
        // A body-sized Paragraph needs one more open-note signal than a Table:
        // the previous page's N definition must visibly end at a discretionary
        // hyphen. This is the source-backed `Intro-` / `duce` shape and keeps a
        // complete body paragraph out even if a segment prior is overbroad.
        if paragraph_segment
            && previous_page_note
                .and_then(|(_, block_index)| blocks.get(block_index))
                .is_none_or(|block| !block.text.trim_end().ends_with('-'))
        {
            continue;
        }
        if near_rule_paragraph_segment
            && previous_page_note
                .and_then(|(_, block_index)| blocks.get(block_index))
                .is_none_or(|previous| {
                    !lm2_reflow_paragraph_is_visibly_open(&previous.text)
                        || !lm2_reflow_starts_like_continuation(&block.text)
                })
        {
            continue;
        }
        if source_marginalia_carryover
            && previous_page_note
                .and_then(|(_, block_index)| blocks.get(block_index))
                .is_none_or(|previous| {
                    !(lm2_reflow_paragraph_is_visibly_open(&previous.text)
                        && lm2_reflow_starts_like_continuation(&block.text)
                        || lm2_citation_suffix_cross_page_bridge(&previous.text, &block.text))
                })
        {
            continue;
        }
        if let Some(block) = blocks.get_mut(source.block_index) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
            if paragraph_segment || near_rule_paragraph_segment {
                paragraph_segment_repairs.insert(source.block_index);
            }
            repaired += 1;
        }
    }
    for source in sources.iter_mut() {
        if paragraph_segment_repairs.contains(&source.block_index) {
            for line in &mut source.lines {
                line.role = LiquidBlockRole::Marginalia;
            }
        }
    }
    repaired
}

pub(super) fn lm2_near_rule_cross_page_note_segment(
    source: &LiquidBlockSourceLines,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let deep = source
        .lines
        .iter()
        .filter_map(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
        })
        .collect::<Vec<_>>();
    let Some(first) = deep.first().copied() else {
        return false;
    };
    deep.len() >= 2
        && source.lines.iter().all(|line| {
            matches!(
                line.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::Footnote
            ) && line.note_markers.is_empty()
        })
        && first.page_object_thin_horizontal_near_line_count > 0
        && first.dist_to_nearest_rule <= first.font_height.max(1.0)
        && deep.iter().all(|line| {
            line.page_index == first.page_index
                && line.segment_block_id == first.segment_block_id
                && line.segment_block_shape.eq_ignore_ascii_case("body")
                && !line.in_footnote_zone
                && !line.in_ruled_cell
                && !line.ruled_row_membership_exact
                && !line.page_table_column_like
                && !line.page_object_overlaps_image_bbox
                && [
                    line.font_ratio_page_ref,
                    line.font_ratio_page,
                    line.font_ratio_doc,
                ]
                .into_iter()
                .filter(|ratio| *ratio <= 0.90)
                .count()
                    >= 2
        })
}

pub(super) fn lm2_citation_suffix_cross_page_bridge(previous: &str, continuation: &str) -> bool {
    let previous = strip_callout_sentinels_lm2(previous);
    let previous = previous.trim_end();
    let continuation = collapse_whitespace(continuation);
    let normalized = continuation
        .replace('\u{2019}', "'")
        .replace('\u{2018}', "'");
    let lower = normalized.to_ascii_lowercase();
    let suffix = lower.starts_with("p'ship,")
        || lower.starts_with("partnership,")
        || lower.starts_with("l.l.c.,")
        || lower.starts_with("llc,")
        || lower.starts_with("ltd.,")
        || lower.starts_with("inc.,");
    let reporter = lower
        .split_whitespace()
        .take(12)
        .any(|token| matches!(token, "f.2d" | "f.3d" | "f.4th" | "u.s." | "s.ct."));
    suffix
        && reporter
        && ["Ltd.", "L.L.C.", "LLC", "Inc.", "P'ship", "P\u{2019}ship"]
            .iter()
            .any(|ending| previous.ends_with(ending))
}

pub(super) fn lm2_body_sized_reflow_line(line: &DeepLiquidSourceLine) -> bool {
    if line.in_ruled_cell
        || line.ruled_row_membership_exact
        || line.page_table_column_like
        || line.page_object_overlaps_image_bbox
    {
        return false;
    }
    [
        line.font_ratio_page_ref,
        line.font_ratio_page,
        line.font_ratio_doc,
    ]
    .into_iter()
    .filter(|ratio| *ratio >= 0.94)
    .count()
        >= 2
}

pub(super) fn lm2_explicit_definition_markers(
    sources: &[LiquidBlockSourceLines],
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> HashSet<u16> {
    sources
        .iter()
        .flat_map(|source| &source.lines)
        .filter(|line| {
            let trimmed = line.text.trim_start();
            let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
            let explicit_punctuation = trimmed[digits..]
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '.' | ')' | ']'));
            explicit_punctuation
                || line
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
                    .is_some_and(|line| {
                        lm2_note_head_zone_evidence(line) && !lm2_body_sized_reflow_line(line)
                    })
        })
        .flat_map(|line| line.note_markers.iter().copied())
        .collect()
}

/// A body row containing one or more superscript callouts can be labelled as a
/// note head when the row is the first item above the real footnote band.  Move
/// it back into the immediately preceding paragraph only when typography or
/// exact same-row geometry says body, and every marker has a real definition.
pub(super) fn apply_inline_body_callout_marginalia_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if blocks.len() < 2 {
        return 0;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let definitions = lm2_explicit_definition_markers(sources, &line_by_id);
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut repaired = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        let candidate = if index + 1 < old_blocks.len()
            && matches!(
                old_blocks[index].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            )
            && matches!(
                old_blocks[index + 1].role,
                LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
            ) {
            let previous_refs = source_by_block.get(&index).cloned().unwrap_or_default();
            let current_refs = source_by_block
                .get(&(index + 1))
                .cloned()
                .unwrap_or_default();
            let mut markers = sentineled_note_markers(&old_blocks[index + 1].text);
            if markers.is_empty() {
                markers.extend(
                    current_refs
                        .iter()
                        .flat_map(|line| line.note_markers.iter().copied()),
                );
                markers.sort_unstable();
                markers.dedup();
            }
            if let (Some(previous_ref), [current_ref]) =
                (previous_refs.last(), current_refs.as_slice())
                && previous_ref.page_index == current_ref.page_index
                && previous_ref.line_index.checked_add(1) == Some(current_ref.line_index)
                && !markers.is_empty()
                && markers.iter().all(|marker| definitions.contains(marker))
                && let (Some(previous_line), Some(current_line)) = (
                    previous_ref
                        .id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied()),
                    current_ref
                        .id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied()),
                )
                && let Some(leading_marker) = leading_sentineled_marker(&old_blocks[index + 1].text)
                    .or_else(|| leading_numeric_token_marker(&old_blocks[index + 1].text))
                && (lm2_body_sized_reflow_line(current_line)
                    || lm2_same_row_leading_callout_with_prose(
                        previous_line,
                        current_line,
                        leading_marker,
                    ))
                && lm2_strip_leading_ascii_or_decoded_callout(
                    &old_blocks[index + 1].text,
                    leading_marker,
                )
                .is_some()
            {
                Some((previous_refs, current_refs))
            } else {
                None
            }
        } else {
            None
        };

        let block_index = rebuilt_blocks.len();
        if let Some((mut previous_refs, mut current_refs)) = candidate {
            let mut merged = old_blocks[index].clone();
            let candidate_text = leading_sentineled_marker(&old_blocks[index + 1].text)
                .map(|_| old_blocks[index + 1].text.clone())
                .or_else(|| {
                    let marker = leading_numeric_token_marker(&old_blocks[index + 1].text)?;
                    let remainder = lm2_strip_leading_ascii_or_decoded_callout(
                        &old_blocks[index + 1].text,
                        marker,
                    )?;
                    Some(format!("{CALLOUT_START}{marker}{CALLOUT_END} {remainder}"))
                })
                .unwrap_or_else(|| old_blocks[index + 1].text.clone());
            append_line(&mut merged.text, &candidate_text);
            for line in &mut current_refs {
                line.role = LiquidBlockRole::Paragraph;
                line.note_markers.clear();
            }
            previous_refs.append(&mut current_refs);
            rebuilt_blocks.push(merged);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: previous_refs,
            });
            source_by_block.remove(&index);
            source_by_block.remove(&(index + 1));
            repaired += 1;
            index += 2;
            continue;
        }

        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }
    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

/// Join a body-sized continuation that sits between an open paragraph and the
/// next paragraph's leading callout.  Exact source-line adjacency plus a real
/// definition lets this cover table/marginalia misroutes without swallowing a
/// genuine note continuation.
pub(super) fn apply_geometry_backed_leading_callout_bridge(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if blocks.len() < 3 {
        return 0;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let definitions = lm2_explicit_definition_markers(sources, &line_by_id);
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut repaired = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        let bridge = if index + 2 < old_blocks.len()
            && old_blocks[index].role == LiquidBlockRole::Paragraph
            && matches!(
                old_blocks[index + 1].role,
                LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote | LiquidBlockRole::Table
            )
            && old_blocks[index + 2].role == LiquidBlockRole::Paragraph
            && lm2_reflow_paragraph_is_visibly_open(&old_blocks[index].text)
            && leading_numeric_token_marker(&old_blocks[index + 1].text).is_none()
            && leading_sentineled_marker(&old_blocks[index + 1].text).is_none()
        {
            let before_refs = source_by_block.get(&index).cloned().unwrap_or_default();
            let bridge_refs = source_by_block
                .get(&(index + 1))
                .cloned()
                .unwrap_or_default();
            let after_refs = source_by_block
                .get(&(index + 2))
                .cloned()
                .unwrap_or_default();
            let marker = leading_sentineled_marker(&old_blocks[index + 2].text);
            if let (
                Some(before_ref),
                Some(bridge_first),
                Some(bridge_last),
                Some(after_ref),
                Some(marker),
            ) = (
                before_refs.last(),
                bridge_refs.first(),
                bridge_refs.last(),
                after_refs.first(),
                marker,
            ) && definitions.contains(&marker)
                && bridge_refs.iter().all(|line| line.note_markers.is_empty())
                && before_ref.page_index == bridge_first.page_index
                && bridge_first.page_index == bridge_last.page_index
                && bridge_last.page_index == after_ref.page_index
                && before_ref.line_index.checked_add(1) == Some(bridge_first.line_index)
                && bridge_last.line_index.checked_add(1) == Some(after_ref.line_index)
                && bridge_refs.iter().all(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied())
                        .is_some_and(lm2_body_sized_reflow_line)
                })
            {
                Some((before_refs, bridge_refs, after_refs, marker))
            } else {
                None
            }
        } else {
            None
        };

        let block_index = rebuilt_blocks.len();
        if let Some((mut before_refs, mut bridge_refs, after_refs, marker)) = bridge {
            let mut merged = old_blocks[index].clone();
            append_line(&mut merged.text, &old_blocks[index + 1].text);
            merged
                .text
                .push_str(&format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
            for line in &mut bridge_refs {
                line.role = LiquidBlockRole::Paragraph;
                line.note_markers.clear();
            }
            before_refs.append(&mut bridge_refs);
            rebuilt_blocks.push(merged);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: before_refs,
            });

            let marker_end = leading_callout_marker_end(&old_blocks[index + 2].text)
                .expect("geometry bridge requires a leading callout");
            let mut continuation = old_blocks[index + 2].clone();
            continuation.text = continuation.text[marker_end..].trim_start().to_owned();
            let continuation_index = rebuilt_blocks.len();
            rebuilt_blocks.push(continuation);
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index: continuation_index,
                lines: after_refs,
            });
            source_by_block.remove(&index);
            source_by_block.remove(&(index + 1));
            source_by_block.remove(&(index + 2));
            repaired += 1;
            index += 3;
            continue;
        }

        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }
    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

/// Split the rare block where a same-row body superscript and its following
/// words were classified Marginalia immediately before the page's real note
/// continuation.  Reading order then groups both regions together (for
/// example `29 If sovereign ide-` followed by the continuation of note 22).
/// Move only the first body line back to its paragraph and preserve the rest as
/// note material.
pub(super) fn apply_mixed_marginalia_leading_body_callout_split(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_position = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let note_pages = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().flat_map(|line| {
                line.note_markers
                    .iter()
                    .map(move |marker| (line.page_index, *marker))
            })
        })
        .collect::<HashSet<_>>();
    let mut operations = Vec::new();

    for block_index in 1..blocks.len() {
        if blocks[block_index - 1].role != LiquidBlockRole::Paragraph
            || blocks[block_index].role != LiquidBlockRole::Marginalia
        {
            continue;
        }
        let (Some(previous_position), Some(current_position)) = (
            source_position.get(&(block_index - 1)).copied(),
            source_position.get(&block_index).copied(),
        ) else {
            continue;
        };
        let previous_source = &sources[previous_position];
        let current_source = &sources[current_position];
        if current_source.lines.len() < 2 {
            continue;
        }
        let (Some(previous_ref), Some(first_ref)) =
            (previous_source.lines.last(), current_source.lines.first())
        else {
            continue;
        };
        if !first_ref.note_markers.is_empty() {
            continue;
        }
        let (Some(previous_line), Some(first_line)) = (
            previous_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
            first_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied()),
        ) else {
            continue;
        };
        let Some(marker) = leading_sentineled_marker(&first_ref.text)
            .or_else(|| leading_numeric_token_marker(&first_ref.text))
        else {
            continue;
        };
        let previous_markers = sentineled_note_markers(&blocks[block_index - 1].text);
        let body_sequence_bracket = previous_markers.contains(&marker.saturating_sub(1))
            && previous_markers.contains(&marker.saturating_add(1));
        if !note_pages.contains(&(first_ref.page_index, marker))
            || (!body_sequence_bracket
                && (lm2_note_head_zone_evidence(first_line)
                    || !lm2_same_row_leading_callout_with_prose(previous_line, first_line, marker)))
        {
            continue;
        }
        let later_note_material = current_source.lines[1..].iter().any(|line| {
            if !line.note_markers.is_empty() {
                return true;
            }
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(|line| {
                    lm2_low_footnote_zone_evidence(line) || lm2_note_head_zone_evidence(line)
                })
        });
        if !later_note_material {
            continue;
        }
        let Some(remainder) =
            lm2_strip_leading_ascii_or_decoded_callout(&first_ref.text, marker).map(str::to_owned)
        else {
            continue;
        };
        operations.push((
            block_index,
            previous_position,
            current_position,
            marker,
            remainder,
            clean_lm2_line_text(&previous_ref.text),
        ));
    }

    for (block_index, previous_position, current_position, marker, remainder, previous_text) in
        operations.iter().cloned()
    {
        let insertion = format!(
            "{CALLOUT_START}{marker}{CALLOUT_END} {}",
            clean_lm2_line_text(&remainder)
        );
        if let Some(start) = blocks[block_index - 1].text.rfind(&previous_text) {
            let end = start + previous_text.len();
            blocks[block_index - 1].text.insert_str(end, &insertion);
        } else {
            blocks[block_index - 1].text =
                format!("{}{}", blocks[block_index - 1].text.trim_end(), insertion);
        }

        let mut first_ref = sources[current_position].lines.remove(0);
        first_ref.role = LiquidBlockRole::Paragraph;
        first_ref.note_markers.clear();
        sources[previous_position].lines.push(first_ref);

        let remaining_refs = sources[current_position].lines.clone();
        let mut remaining_text = String::new();
        for line in &remaining_refs {
            append_line(&mut remaining_text, &line.text);
        }
        blocks[block_index].text = collapse_whitespace(&remaining_text).trim().to_owned();
    }

    operations.len()
}

/// A mixed Marginalia split can move `N Prose` into a body block whose text
/// already contains later-page continuation text. Reinsert that fragment after
/// its exact physical predecessor when the body itself proves N-1/N/N+1 and N
/// currently appears after N+1.
pub(super) fn apply_terminal_out_of_order_callout_fragment_reflow(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let line_by_coordinate = decoded
        .iter()
        .map(|(line, _)| ((line.page_index, line.line_index), line))
        .collect::<HashMap<_, _>>();
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;

    for (block_index, block) in blocks.iter_mut().enumerate() {
        if !matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        ) {
            continue;
        }
        let markers = sentineled_note_markers(&block.text);
        let Some(source) = source_by_block.get(&block_index).copied() else {
            continue;
        };
        for line_ref in &source.lines {
            let Some(marker) = leading_sentineled_marker(&line_ref.text)
                .or_else(|| leading_numeric_token_marker(&line_ref.text))
            else {
                continue;
            };
            if !markers.contains(&marker.saturating_sub(1))
                || !markers.contains(&marker)
                || !markers.contains(&marker.saturating_add(1))
            {
                continue;
            }
            let Some(current) = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let Some(previous_line_index) = current.line_index.checked_sub(1) else {
                continue;
            };
            let Some(previous) = line_by_coordinate
                .get(&(current.page_index, previous_line_index))
                .copied()
            else {
                continue;
            };
            let Some(remainder) =
                lm2_strip_leading_ascii_or_decoded_callout(&line_ref.text, marker)
            else {
                continue;
            };
            let rendered = format!(
                "{CALLOUT_START}{marker}{CALLOUT_END} {}",
                clean_lm2_line_text(remainder)
            );
            let Some(fragment_start) = block.text.rfind(&rendered) else {
                continue;
            };
            let next_marker = format!("{CALLOUT_START}{}{CALLOUT_END}", marker + 1);
            let Some(next_start) = block.text.find(&next_marker) else {
                continue;
            };
            let anchor = clean_lm2_line_text(&previous.text);
            let Some(anchor_start) = block.text.rfind(&anchor) else {
                continue;
            };
            let anchor_end = anchor_start + anchor.len();
            if fragment_start < next_start || fragment_start <= anchor_end {
                continue;
            }
            let fragment_end = fragment_start + rendered.len();
            let mut rewritten = block.text.clone();
            rewritten.replace_range(fragment_start..fragment_end, "");
            rewritten.insert_str(anchor_end, &rendered);
            block.text = collapse_whitespace(&rewritten).trim().to_owned();
            repaired += 1;
            break;
        }
    }
    repaired
}

/// Do not let a page-level marker sequence consume the middle component of a
/// dotted software/version number (`V3.3.2`). The original source row provides
/// an exact positive reconstruction, so ordinary decimals and real callouts
/// remain untouched.
pub(super) fn apply_source_backed_dotted_numeric_callout_restoration(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
) -> usize {
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    for (block_index, block) in blocks.iter_mut().enumerate() {
        if !matches!(
            block.role,
            LiquidBlockRole::Paragraph
                | LiquidBlockRole::Lead
                | LiquidBlockRole::Quote
                | LiquidBlockRole::ListItem
        ) {
            continue;
        }
        let markers = sentineled_note_markers(&block.text);
        if markers.is_empty() {
            continue;
        }
        let Some(source) = source_by_block.get(&block_index).copied() else {
            continue;
        };
        for line_ref in &source.lines {
            if line_ref.text.contains(CALLOUT_START) {
                continue;
            }
            let clean = clean_lm2_line_text(&line_ref.text);
            let bytes = clean.as_bytes();
            let mut position = 0usize;
            while position < bytes.len() {
                if !bytes[position].is_ascii_digit() {
                    position += 1;
                    continue;
                }
                let start = position;
                while position < bytes.len() && bytes[position].is_ascii_digit() {
                    position += 1;
                }
                let end = position;
                if start == 0
                    || end >= bytes.len()
                    || bytes[start - 1] != b'.'
                    || bytes[end] != b'.'
                {
                    continue;
                }
                for marker in &markers {
                    let mut corrupted = clean.clone();
                    corrupted.replace_range(
                        start..end,
                        &format!("{CALLOUT_START}{marker}{CALLOUT_END}"),
                    );
                    if block.text.contains(&corrupted) {
                        block.text = block.text.replacen(&corrupted, &clean, 1);
                        repaired += 1;
                        break;
                    }
                }
            }
        }
    }
    repaired
}

/// Rejoin a body line that the line model called Marginalia even though its
/// source provenance is physically contiguous with an open Paragraph. This is
/// especially common for an indented displayed quotation at a page top or for
/// its short final line. Genuine note heads are excluded by source markers.
pub(super) fn apply_contiguous_body_marginalia_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let footnote_zone_ids = decoded
        .iter()
        .filter(|(line, _)| lm2_low_footnote_zone_evidence(line))
        .map(|(line, _)| line.id.as_str())
        .collect::<HashSet<_>>();
    let mut total = 0usize;
    for _ in 0..8 {
        let repaired =
            apply_contiguous_body_marginalia_reflow_once(blocks, sources, &footnote_zone_ids);
        if repaired == 0 {
            break;
        }
        total += repaired;
    }
    total
}

pub(super) fn apply_contiguous_body_marginalia_reflow_once(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    footnote_zone_ids: &HashSet<&str>,
) -> usize {
    if blocks.len() < 2 {
        return 0;
    }
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut repaired = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        let merge_pair = if index + 1 < old_blocks.len()
            && lm2_source_blocks_contiguous(&source_by_block, index, index + 1)
        {
            let left = &old_blocks[index];
            let right = &old_blocks[index + 1];
            let left_false_marginalia =
                lm2_false_body_marginalia(left, source_by_block.get(&index), footnote_zone_ids)
                    && right.role == LiquidBlockRole::Paragraph
                    && left.text.split_whitespace().count() >= 4
                    && lm2_reflow_paragraph_is_visibly_open(&left.text)
                    && lm2_reflow_starts_like_continuation(&right.text);
            let right_false_marginalia = left.role == LiquidBlockRole::Paragraph
                && lm2_false_body_marginalia(
                    right,
                    source_by_block.get(&(index + 1)),
                    footnote_zone_ids,
                )
                && lm2_reflow_paragraph_is_visibly_open(&left.text)
                && lm2_reflow_starts_like_continuation(&right.text);
            left_false_marginalia || right_false_marginalia
        } else {
            false
        };

        let block_index = rebuilt_blocks.len();
        if merge_pair {
            let mut merged = old_blocks[index].clone();
            merged.role = LiquidBlockRole::Paragraph;
            append_line(&mut merged.text, &old_blocks[index + 1].text);
            rebuilt_blocks.push(merged);
            let mut refs = source_by_block.remove(&index).unwrap_or_default();
            refs.extend(source_by_block.remove(&(index + 1)).unwrap_or_default());
            if !refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: refs,
                });
            }
            repaired += 1;
            index += 2;
            continue;
        }

        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }

    if repaired > 0 {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    } else {
        *blocks = old_blocks;
    }
    repaired
}

pub(super) fn lm2_false_body_marginalia(
    block: &LiquidBlock,
    source_lines: Option<&Vec<LiquidSourceLineRef>>,
    footnote_zone_ids: &HashSet<&str>,
) -> bool {
    matches!(
        block.role,
        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
    ) && source_lines.is_some_and(|lines| {
        !lines.is_empty()
            && lines.iter().all(|line| {
                line.note_markers.is_empty()
                    && line
                        .id
                        .as_deref()
                        .is_none_or(|id| !footnote_zone_ids.contains(id))
            })
    })
}

pub(super) fn lm2_source_blocks_contiguous(
    sources: &BTreeMap<usize, Vec<LiquidSourceLineRef>>,
    left_index: usize,
    right_index: usize,
) -> bool {
    let (Some(left), Some(right)) = (sources.get(&left_index), sources.get(&right_index)) else {
        return false;
    };
    let (Some(left_line), Some(right_line)) = (left.last(), right.first()) else {
        return false;
    };
    left_line.page_index == right_line.page_index
        && left_line.line_index.checked_add(1) == Some(right_line.line_index)
}

/// Undo recurrent false footnote heads created by flattened citation text:
/// a URL filename split after a hyphen (`...10-5-` + `15.pdf`) and a journal
/// page followed by a parenthesized year (`REV.` + `99 (2020)`). A third shape
/// is a pincite followed by citation comparison text (`at` + `232. Compare`).
/// All shapes require neighboring, provenance-backed note heads before the
/// false marker is removed and the continuation is restored.
pub(super) fn apply_false_marginalia_note_head_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    if blocks.len() < 3 {
        return 0;
    }
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut repaired = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        let source_backed_previous_note = old_blocks[index].role == LiquidBlockRole::Noise
            && source_by_block.get(&index).is_some_and(|lines| {
                !lines.is_empty()
                    && lines
                        .iter()
                        .all(|line| line.role == LiquidBlockRole::Marginalia)
            });
        let false_head = if index + 2 < old_blocks.len()
            && (old_blocks[index].role == LiquidBlockRole::Marginalia
                || (old_blocks[index].role == LiquidBlockRole::Noise
                    && source_backed_previous_note))
            && old_blocks[index + 1].role == LiquidBlockRole::Marginalia
            && old_blocks[index + 2].role == LiquidBlockRole::Marginalia
        {
            let previous_marker = lm2_source_primary_note_marker(source_by_block.get(&index));
            let current_marker = lm2_source_primary_note_marker(source_by_block.get(&(index + 1)));
            let next_marker = lm2_source_primary_note_marker(source_by_block.get(&(index + 2)));
            current_marker.is_some_and(|marker| {
                let current = old_blocks[index + 1].text.trim_start();
                let digits = marker.to_string();
                let tail = current
                    .strip_prefix(&digits)
                    .map(str::trim_start)
                    .unwrap_or("");
                let url_filename = old_blocks[index].text.trim_end().ends_with('-')
                    && tail.to_ascii_lowercase().starts_with(".pdf");
                let citation_page = lm2_starts_parenthesized_year(tail)
                    && previous_marker
                        .zip(next_marker)
                        .is_some_and(|(previous, next)| {
                            next == previous.saturating_add(1) && marker > next && marker <= 999
                        });
                let compare_pincite =
                    previous_marker
                        .zip(next_marker)
                        .is_some_and(|(previous, next)| {
                            let after_punctuation = tail
                                .strip_prefix('.')
                                .or_else(|| tail.strip_prefix(')'))
                                .or_else(|| tail.strip_prefix(']'))
                                .map(str::trim_start)
                                .unwrap_or(tail);
                            next == previous.saturating_add(1)
                                && marker != next
                                && old_blocks[index]
                                    .text
                                    .trim_end()
                                    .to_ascii_lowercase()
                                    .ends_with(" at")
                                && after_punctuation
                                    .to_ascii_lowercase()
                                    .starts_with("compare ")
                        });
                let standalone_pincite =
                    previous_marker
                        .zip(next_marker)
                        .is_some_and(|(previous, next)| {
                            next == previous.saturating_add(1)
                                && marker != next
                                && tail.trim() == "."
                                && (old_blocks[index]
                                    .text
                                    .trim_end()
                                    .ends_with(['-', '\u{2013}', '\u{2014}'])
                                    || old_blocks[index]
                                        .text
                                        .trim_end()
                                        .to_ascii_lowercase()
                                        .ends_with("supra note")
                                    || old_blocks[index]
                                        .text
                                        .trim_end()
                                        .to_ascii_lowercase()
                                        .ends_with(" at")
                                    || old_blocks[index]
                                        .text
                                        .trim_end()
                                        .to_ascii_lowercase()
                                        .ends_with("dkt. no."))
                        });
                url_filename || citation_page || compare_pincite || standalone_pincite
            })
        } else {
            false
        };

        let block_index = rebuilt_blocks.len();
        if false_head {
            let mut merged = old_blocks[index].clone();
            // A false out-of-sequence pincite can make the sequence decoder
            // demote the real preceding note. Its source provenance and the
            // surrounding N/N+1 heads make that demotion safe to reverse.
            merged.role = LiquidBlockRole::Marginalia;
            let continuation = old_blocks[index + 1].text.trim_start();
            let preserves_url_hyphen = merged.text.trim_end().ends_with('-')
                && lm2_source_primary_note_marker(source_by_block.get(&(index + 1))).is_some_and(
                    |marker| {
                        continuation
                            .strip_prefix(&marker.to_string())
                            .is_some_and(|tail| tail.trim_start().starts_with(".pdf"))
                    },
                );
            if preserves_url_hyphen {
                merged.text.push_str(continuation);
            } else if continuation.trim_end() == "."
                && merged
                    .text
                    .trim_end()
                    .ends_with(['-', '\u{2013}', '\u{2014}'])
            {
                merged.text.push_str(continuation);
            } else {
                append_line(&mut merged.text, continuation);
            }
            rebuilt_blocks.push(merged);
            let mut refs = source_by_block.remove(&index).unwrap_or_default();
            let mut continuation_refs = source_by_block.remove(&(index + 1)).unwrap_or_default();
            for line in &mut continuation_refs {
                line.note_markers.clear();
            }
            refs.extend(continuation_refs);
            if !refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: refs,
                });
            }
            repaired += 1;
            index += 2;
            continue;
        }

        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_source_primary_note_marker(
    lines: Option<&Vec<LiquidSourceLineRef>>,
) -> Option<u16> {
    lines?
        .iter()
        .find_map(|line| line.note_markers.first().copied())
}

pub(super) fn lm2_starts_parenthesized_year(text: &str) -> bool {
    let Some(year) = text.strip_prefix('(').and_then(|tail| tail.get(..4)) else {
        return false;
    };
    year.chars().all(|ch| ch.is_ascii_digit())
        && year
            .parse::<u16>()
            .is_ok_and(|year| (1500..=2200).contains(&year))
        && text.get(5..6) == Some(")")
}

/// PDF text layers occasionally flatten a superscript into an attached ASCII
/// suffix (`initiated.128`). Recover the callout only when source provenance
/// also contains a matching note head on the same or following page.
pub(super) fn apply_attached_terminal_callout_recovery(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
) -> usize {
    let mut note_pages: BTreeMap<u16, HashSet<usize>> = BTreeMap::new();
    for source in sources {
        for line in &source.lines {
            for marker in &line.note_markers {
                note_pages
                    .entry(*marker)
                    .or_default()
                    .insert(line.page_index);
            }
        }
    }

    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<BTreeMap<_, _>>();
    let mut repaired = 0usize;
    for (block_index, block) in blocks.iter_mut().enumerate() {
        let ordinary_body_role = matches!(
            block.role,
            LiquidBlockRole::Paragraph
                | LiquidBlockRole::Lead
                | LiquidBlockRole::Quote
                | LiquidBlockRole::ListItem
        );
        let table_body_role = matches!(
            block.role,
            LiquidBlockRole::Table | LiquidBlockRole::Caption
        );
        let Some(source) = source_by_block.get(&block_index) else {
            continue;
        };
        let unmarked_body_candidate =
            matches!(
                block.role,
                LiquidBlockRole::Noise | LiquidBlockRole::Marginalia
            ) && source.lines.iter().all(|line| line.note_markers.is_empty());
        if !ordinary_body_role && !table_body_role && !unmarked_body_candidate {
            continue;
        }
        for line in &source.lines {
            let cleaned = clean_lm2_line_text(&line.text);
            let mut ranges = Vec::new();
            if table_body_role {
                for (marker, pages) in &note_pages {
                    if !(pages.contains(&line.page_index)
                        || pages.contains(&line.page_index.saturating_add(1)))
                    {
                        continue;
                    }
                    let mut from = 0usize;
                    while let Some((start, end)) =
                        attached_ascii_callout_range(&cleaned, *marker, from)
                    {
                        ranges.push((start, end, *marker));
                        from = end;
                    }
                }
            } else if let Some((marker, span)) = attached_terminal_ascii_marker_span(&cleaned)
                && note_pages.get(&marker).is_some_and(|pages| {
                    pages.contains(&line.page_index)
                        || pages.contains(&line.page_index.saturating_add(1))
                })
            {
                ranges.push((span.start, span.end, marker));
            }
            if ranges.is_empty() {
                continue;
            }
            ranges.sort_by_key(|(start, _, _)| *start);
            ranges.dedup();
            let mut replacement = cleaned.clone();
            for (start, end, marker) in ranges.iter().copied().rev() {
                replacement
                    .replace_range(start..end, &format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
            }
            if block.text.contains(&cleaned) {
                block.text = block.text.replacen(&cleaned, &replacement, 1);
                if unmarked_body_candidate {
                    block.role = LiquidBlockRole::Paragraph;
                    block.label = None;
                }
                repaired += ranges.len();
            }
        }
    }
    repaired
}

pub(super) fn lm2_physical_sequential_note_row(line: &DeepLiquidSourceLine) -> bool {
    let small_font_votes = [
        line.font_ratio_page_ref <= 0.94,
        line.font_ratio_page <= 0.94,
        line.font_ratio_doc <= 0.94,
    ]
    .into_iter()
    .filter(|vote| *vote)
    .count();
    let ordinary_note_zone = if line.page_has_footnote_divider {
        line.below_footnote_divider
    } else {
        (line.in_footnote_zone || line.doc_footnote_state)
            && (line.segment_block_footnote_like
                || line.segment_block_shape.eq_ignore_ascii_case("footnote"))
    };
    // A divider can be geometrically assigned to the wrong horizontal rule.
    // Retain a tiny, non-centred Marginalia row as a candidate so the exact
    // N-1/N+1 sequence and following note prose can decide it later.
    let missed_divider_note = line.role_hint == Some(LiquidBlockRole::Marginalia)
        && line.font_ratio_doc <= 0.70
        && !line.centered
        && line.line_index > 0
        && (line.in_footnote_zone
            || line.segment_block_footnote_like
            || line.font_ratio_page_ref <= 0.70);
    (ordinary_note_zone || missed_divider_note)
        && small_font_votes >= 2
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && (!line.segment_block_table_like || missed_divider_note)
        && !line.page_object_overlaps_image_bbox
        && !line.page_object_hide_candidate
        && (!line.segment_block_furniture_like || missed_divider_note)
        && !line.doc_repeated_edge_text
}

/// A matching body callout supplies stronger semantic evidence than a local
/// N-1/N+1 sequence. In that paired case, retain tiny, non-centred Marginalia
/// rows even when the divider or segment detector assigned the rule poorly.
pub(super) fn lm2_body_paired_bare_note_row(line: &DeepLiquidSourceLine) -> bool {
    let small_font_votes = [
        line.font_ratio_page_ref <= 0.94,
        line.font_ratio_page <= 0.94,
        line.font_ratio_doc <= 0.94,
    ]
    .into_iter()
    .filter(|vote| *vote)
    .count();
    line.font_ratio_doc <= 0.70
        && small_font_votes >= 2
        && !line.centered
        && line.line_index > 0
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
        && !line.page_object_hide_candidate
        && !line.doc_repeated_edge_text
}

pub(super) fn lm2_body_paired_note_prose_row(line: &DeepLiquidSourceLine) -> bool {
    lm2_body_paired_bare_note_row(line)
        || (line.font_ratio_doc <= 0.82
            && !line.in_ruled_cell
            && !line.ruled_row_membership_exact
            && !line.page_table_column_like
            && !line.page_object_overlaps_image_bbox
            && !line.page_object_hide_candidate
            && !line.doc_repeated_edge_text)
}

pub(super) fn lm2_leading_numeric_or_bare_marker(text: &str) -> Option<u16> {
    let trimmed = text.trim();
    trimmed
        .parse::<u16>()
        .ok()
        .filter(|marker| (1..=LM2_MAX_NOTE_MARKER).contains(marker))
        .or_else(|| leading_numeric_token_marker(text))
}

pub(super) fn lm2_note_tail_visibly_closed(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', '\u{2019}', '\u{201d}', ')', ']', '}'])
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ')' | ']'))
}

/// Recover bare `N` and `N prose` note heads across grouping-block boundaries.
/// The marker must be the exact successor of an accepted physical note head,
/// occur within eight source rows, and retain small-font footnote geometry.
/// This excludes page numbers, reporter volumes, years, and table counters
/// without depending on document-specific words.
pub(super) fn apply_physical_sequential_note_head_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut rows = sources
        .iter()
        .enumerate()
        .flat_map(|(source_position, source)| {
            source
                .lines
                .iter()
                .enumerate()
                .map(move |(line_position, line)| {
                    (
                        line.page_index,
                        line.line_index,
                        source_position,
                        line_position,
                    )
                })
        })
        .collect::<Vec<_>>();
    rows.sort_unstable();

    let mut accepted = rows
        .iter()
        .enumerate()
        .filter_map(|(row_position, &(_, _, source_position, line_position))| {
            sources[source_position].lines[line_position]
                .note_markers
                .iter()
                .copied()
                .max()
                .map(|marker| (row_position, marker))
        })
        .collect::<BTreeMap<_, _>>();
    let mut repairs = Vec::<(usize, usize, u16)>::new();
    loop {
        let mut pass_repairs = Vec::new();
        for (row_position, &(page_index, line_index, source_position, line_position)) in
            rows.iter().enumerate()
        {
            if accepted.contains_key(&row_position) {
                continue;
            }
            let line_ref = &sources[source_position].lines[line_position];
            let Some(deep) = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let Some(candidate) = lm2_leading_numeric_or_bare_marker(&line_ref.text) else {
                continue;
            };
            if !lm2_physical_sequential_note_row(deep) {
                continue;
            }
            let previous_proof =
                accepted
                    .range(..row_position)
                    .next_back()
                    .is_some_and(|(position, marker)| {
                        row_position.saturating_sub(*position) <= 16
                            && *marker == candidate.saturating_sub(1)
                            && rows[*position].0.abs_diff(page_index) <= 1
                    });
            let next_proof =
                accepted
                    .range((row_position + 1)..)
                    .next()
                    .is_some_and(|(position, marker)| {
                        position.saturating_sub(row_position) <= 16
                            && *marker == candidate.saturating_add(1)
                            && rows[*position].0.abs_diff(page_index) <= 1
                    });
            if !previous_proof && !next_proof {
                continue;
            }
            let bare_marker = line_ref.text.trim() == candidate.to_string();
            let bare_followed_by_note_prose = bare_marker
                && rows.get(row_position + 1).is_some_and(
                    |&(next_page, next_line, next_source, next_position)| {
                        if next_page != page_index || next_line != line_index.saturating_add(1) {
                            return false;
                        }
                        let next_ref = &sources[next_source].lines[next_position];
                        next_ref
                            .id
                            .as_deref()
                            .and_then(|id| line_by_id.get(id).copied())
                            .is_some_and(|next| {
                                lm2_physical_sequential_note_row(next)
                                    && lm2_leading_numeric_or_bare_marker(&next_ref.text).is_none()
                                    && next_ref.text.chars().any(char::is_alphabetic)
                                    && deep.font_height.max(next.font_height)
                                        / deep.font_height.min(next.font_height).max(1.0)
                                        <= 1.18
                            })
                    },
                );
            let previous_text = row_position.checked_sub(1).map(|position| {
                let (_, _, source, line) = rows[position];
                sources[source].lines[line].text.as_str()
            });
            if !bare_followed_by_note_prose
                && previous_text.is_none_or(|text| !lm2_note_tail_visibly_closed(text))
            {
                continue;
            }
            pass_repairs.push((row_position, source_position, line_position, candidate));
        }
        if pass_repairs.is_empty() {
            break;
        }
        for (row_position, source_position, line_position, marker) in pass_repairs {
            accepted.insert(row_position, marker);
            repairs.push((source_position, line_position, marker));
        }
    }

    let mut repaired = 0usize;
    let mut touched_sources = BTreeSet::new();
    for (source_position, line_position, marker) in repairs {
        let line = &mut sources[source_position].lines[line_position];
        line.role = LiquidBlockRole::Marginalia;
        line.note_markers.push(marker);
        line.note_markers.sort_unstable();
        line.note_markers.dedup();
        touched_sources.insert(source_position);
        repaired += 1;
    }
    for source_position in touched_sources {
        let all_note_rows = sources[source_position].lines.iter().all(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(lm2_physical_sequential_note_row)
        });
        if !all_note_rows {
            continue;
        }
        for line in &mut sources[source_position].lines {
            line.role = LiquidBlockRole::Marginalia;
        }
        if let Some(block) = blocks.get_mut(sources[source_position].block_index) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
        }
    }
    repaired
}

/// A grouping boundary can leave a recovered bare note marker at the end of a
/// body/Noise block while its definition prose begins the next Marginalia
/// block. Move only that final physical row across the boundary; the body or
/// URL text that precedes it remains in its original block.
pub(super) fn apply_recovered_trailing_note_marker_reflow(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    loop {
        let source_position = sources
            .iter()
            .enumerate()
            .map(|(position, source)| (source.block_index, position))
            .collect::<HashMap<_, _>>();
        let mut operation = None;
        for block_index in 0..blocks.len().saturating_sub(1) {
            if matches!(
                blocks[block_index].role,
                LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
            ) {
                continue;
            }
            let next_block = block_index + 1;
            let (Some(current_position), Some(next_position)) = (
                source_position.get(&block_index).copied(),
                source_position.get(&next_block).copied(),
            ) else {
                continue;
            };
            let Some(next_ref) = sources[next_position].lines.first() else {
                continue;
            };
            let Some((marker_position, marker_ref)) = sources[current_position]
                .lines
                .iter()
                .enumerate()
                .find(|(_, line)| {
                    line.page_index == next_ref.page_index
                        && line.line_index.saturating_add(1) == next_ref.line_index
                        && line
                            .text
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .is_some_and(|marker| line.note_markers.contains(&marker))
                })
            else {
                continue;
            };
            let marker = marker_ref
                .text
                .trim()
                .parse::<u16>()
                .expect("paired marker row was validated above");
            if marker_ref.text.trim() != marker.to_string()
                || !matches!(
                    blocks[next_block].role,
                    LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
                )
            {
                continue;
            }
            let (Some(marker_line), Some(next_line)) = (
                marker_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
                next_ref
                    .id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied()),
            ) else {
                continue;
            };
            if marker_line.page_index != next_line.page_index
                || marker_line.line_index.saturating_add(1) != next_line.line_index
                || !(lm2_physical_sequential_note_row(marker_line)
                    || lm2_body_paired_bare_note_row(marker_line))
                || next_ref.role != LiquidBlockRole::Marginalia
                || !(lm2_physical_sequential_note_row(next_line)
                    || lm2_body_paired_note_prose_row(next_line))
                || next_ref.note_markers.iter().any(|next| *next <= marker)
            {
                continue;
            }
            operation = Some((
                block_index,
                next_block,
                current_position,
                next_position,
                marker_position,
            ));
            break;
        }
        let Some((block_index, next_block, current_position, next_position, marker_position)) =
            operation
        else {
            break;
        };
        let marker_text =
            clean_lm2_line_text(&sources[current_position].lines[marker_position].text);
        if !remove_unique_whitespace_token(&mut blocks[block_index].text, &marker_text) {
            continue;
        }
        let mut marker_ref = sources[current_position].lines.remove(marker_position);
        marker_ref.role = LiquidBlockRole::Marginalia;
        let next_text = blocks[next_block].text.clone();
        blocks[next_block].text = collapse_whitespace(&format!("{marker_text} {next_text}"));
        blocks[next_block].role = LiquidBlockRole::Marginalia;
        blocks[next_block].label = None;
        sources[next_position].lines.insert(0, marker_ref);
        if blocks[block_index].text.trim().is_empty() {
            blocks[block_index].role = LiquidBlockRole::Noise;
            blocks[block_index].label = None;
        }
        repaired += 1;
    }
    repaired
}

pub(super) fn remove_unique_whitespace_token(text: &mut String, token: &str) -> bool {
    let bytes = text.as_bytes();
    let token_bytes = token.as_bytes();
    if token_bytes.is_empty() || token_bytes.iter().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    let mut matches = Vec::new();
    let mut cursor = 0usize;
    while cursor + token_bytes.len() <= bytes.len() {
        let Some(relative) = text[cursor..].find(token) else {
            break;
        };
        let start = cursor + relative;
        let end = start + token_bytes.len();
        let left_ok = start == 0 || bytes[start - 1].is_ascii_whitespace();
        let right_ok = end == bytes.len() || bytes[end].is_ascii_whitespace();
        if left_ok && right_ok {
            matches.push((start, end));
        }
        cursor = end;
    }
    if matches.len() != 1 {
        return false;
    }
    let (start, end) = matches[0];
    text.replace_range(start..end, "");
    *text = collapse_whitespace(text).trim().to_owned();
    true
}

/// Recover a URL-only note continuation that is physically sandwiched between
/// consecutive numbered definitions. The exact N / URL rows / N+1 source
/// sequence prevents ordinary body URLs from being absorbed into notes.
pub(super) fn apply_sandwiched_note_url_continuation_reflow(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_position = sources
        .iter()
        .enumerate()
        .map(|(position, source)| (source.block_index, position))
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    for block_index in 1..blocks.len().saturating_sub(1) {
        if !matches!(
            blocks[block_index - 1].role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
        ) || !matches!(
            blocks[block_index].role,
            LiquidBlockRole::Noise | LiquidBlockRole::Paragraph
        ) || !matches!(
            blocks[block_index + 1].role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
        ) || !blocks[block_index]
            .text
            .to_ascii_lowercase()
            .contains("http")
        {
            continue;
        }
        let (Some(previous_position), Some(current_position), Some(next_position)) = (
            source_position.get(&(block_index - 1)).copied(),
            source_position.get(&block_index).copied(),
            source_position.get(&(block_index + 1)).copied(),
        ) else {
            continue;
        };
        let (Some(previous_marker), Some(next_marker)) = (
            lm2_source_primary_note_marker(Some(&sources[previous_position].lines)),
            lm2_source_primary_note_marker(Some(&sources[next_position].lines)),
        ) else {
            continue;
        };
        if next_marker != previous_marker.saturating_add(1)
            || sources[current_position].lines.is_empty()
            || sources[current_position]
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty())
        {
            continue;
        }
        let (Some(previous), Some(current_first), Some(current_last), Some(next)) = (
            sources[previous_position].lines.last(),
            sources[current_position].lines.first(),
            sources[current_position].lines.last(),
            sources[next_position].lines.first(),
        ) else {
            continue;
        };
        let physically_sandwiched = previous.page_index == current_first.page_index
            && current_first.page_index == current_last.page_index
            && current_last.page_index == next.page_index
            && previous.line_index.saturating_add(1) == current_first.line_index
            && current_last.line_index.saturating_add(1) == next.line_index;
        let all_small_note_rows = sources[current_position].lines.iter().all(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(|deep| {
                    deep.font_ratio_doc <= 0.90
                        && !deep.in_ruled_cell
                        && !deep.ruled_row_membership_exact
                        && !deep.page_table_column_like
                        && !deep.page_object_overlaps_image_bbox
                })
        });
        if !physically_sandwiched || !all_small_note_rows {
            continue;
        }
        blocks[block_index].role = LiquidBlockRole::Marginalia;
        blocks[block_index].label = None;
        for line in &mut sources[current_position].lines {
            line.role = LiquidBlockRole::Marginalia;
        }
        repaired += 1;
    }
    repaired
}

/// Pair a flattened body callout with a bare physical definition row carrying
/// the same number. This is stronger than local sequence distance: both ends
/// of the link independently exist, while the definition still has to satisfy
/// small-font footnote geometry. Plain ASCII body candidates are limited to
/// punctuation/leading-callout shapes and are rewritten only by the separate
/// monotone callout pass.
pub(super) fn apply_body_backed_bare_note_head_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut body_markers = BTreeSet::new();
    for block in blocks.iter().filter(|block| {
        !matches!(
            block.role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
        )
    }) {
        body_markers.extend(sentineled_note_markers(&block.text));
        body_markers.extend(
            lm2_local_ascii_callout_candidates(&block.text)
                .into_iter()
                .map(|candidate| candidate.marker),
        );
        if let Some(marker) = attached_terminal_ascii_marker(&block.text) {
            body_markers.insert(marker);
        }
    }
    for source in sources.iter() {
        for line_ref in &source.lines {
            let Some(deep) = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            if lm2_low_footnote_zone_evidence(deep)
                || deep.in_ruled_cell
                || deep.ruled_row_membership_exact
                || deep.page_table_column_like
                || deep.segment_block_table_like
                || deep.page_object_overlaps_image_bbox
                || deep.segment_block_furniture_like
                || deep.font_ratio_doc < 0.78
            {
                continue;
            }
            body_markers.extend(sentineled_note_markers(&line_ref.text));
            body_markers.extend(
                lm2_local_ascii_callout_candidates(&line_ref.text)
                    .into_iter()
                    .map(|candidate| candidate.marker),
            );
        }
    }
    if body_markers.is_empty() {
        return 0;
    }

    let mut touched_sources = BTreeSet::new();
    let mut repaired = 0usize;
    for (source_position, source) in sources.iter_mut().enumerate() {
        for line_ref in &mut source.lines {
            if !line_ref.note_markers.is_empty() {
                continue;
            }
            let Some(marker) = line_ref
                .text
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|marker| body_markers.contains(marker))
            else {
                continue;
            };
            let physical = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(|line| {
                    lm2_physical_sequential_note_row(line)
                        || (line_ref.role == LiquidBlockRole::Marginalia
                            && lm2_body_paired_bare_note_row(line))
                });
            if !physical {
                continue;
            }
            line_ref.role = LiquidBlockRole::Marginalia;
            line_ref.note_markers.push(marker);
            touched_sources.insert(source_position);
            repaired += 1;
        }
    }
    for source_position in touched_sources {
        let all_note_rows = sources[source_position].lines.iter().all(|line| {
            line.id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
                .is_some_and(lm2_physical_sequential_note_row)
        });
        if !all_note_rows {
            continue;
        }
        for line in &mut sources[source_position].lines {
            line.role = LiquidBlockRole::Marginalia;
        }
        if let Some(block) = blocks.get_mut(sources[source_position].block_index) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
        }
    }
    repaired
}

#[derive(Clone, Copy)]
pub(super) struct Lm2AsciiCalloutCandidate {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) marker: u16,
    pub(super) leading: bool,
}

pub(super) fn lm2_local_ascii_callout_candidates(text: &str) -> Vec<Lm2AsciiCalloutCandidate> {
    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
    let mut position = 0usize;
    while position < bytes.len() {
        if !bytes[position].is_ascii_digit()
            || position > 0 && !bytes[position - 1].is_ascii_whitespace()
        {
            position += 1;
            continue;
        }
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_digit() && position - start < 3 {
            position += 1;
        }
        let end = position;
        if position < bytes.len() && bytes[position].is_ascii_digit() {
            continue;
        }
        let Ok(marker) = text[start..end].parse::<u16>() else {
            continue;
        };
        if !(1..=LM2_MAX_NOTE_MARKER).contains(&marker)
            || bytes
                .get(end)
                .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            continue;
        }
        let leading = text[..start].trim().is_empty();
        let punctuated = text[..start]
            .trim_end()
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ';' | ':' | ',' | ')' | ']'));
        let remainder_is_prose = text[end..]
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_alphabetic);
        if (leading && remainder_is_prose) || punctuated {
            candidates.push(Lm2AsciiCalloutCandidate {
                start,
                end,
                marker,
                leading,
            });
        }
    }
    candidates
}

/// Recover one whitespace-separated body callout only when accepted note-head
/// provenance and the nearest already-sentineled body callouts bracket it as
/// `N-1, N, N+1`. A leading candidate additionally needs same-row geometry
/// with the preceding body fragment. Only the exact digit token is rewritten.
pub(super) fn apply_local_monotone_ascii_callout_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let accepted_heads = sources
        .iter()
        .flat_map(|source| source.lines.iter())
        .flat_map(|line| line.note_markers.iter().copied())
        .collect::<BTreeSet<_>>();

    #[derive(Clone)]
    struct Occurrence {
        page: usize,
        line: usize,
        offset: usize,
        marker: u16,
        candidate: Option<(usize, usize, bool)>,
    }
    let mut occurrences = Vec::<Occurrence>::new();
    for (source_position, source) in sources.iter().enumerate() {
        for (line_position, line_ref) in source.lines.iter().enumerate() {
            let Some(deep) = line_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            if !line_ref.note_markers.is_empty()
                || lm2_low_footnote_zone_evidence(deep)
                || deep.in_ruled_cell
                || deep.ruled_row_membership_exact
                || deep.page_table_column_like
                || deep.segment_block_table_like
                || deep.page_object_overlaps_image_bbox
                || deep.segment_block_furniture_like
                || deep.font_ratio_doc < 0.78
            {
                continue;
            }
            let mut cursor = 0usize;
            while let Some(relative) = line_ref.text[cursor..].find(CALLOUT_START) {
                let start = cursor + relative;
                let marker_start = start + CALLOUT_START.len_utf8();
                let Some(relative_end) = line_ref.text[marker_start..].find(CALLOUT_END) else {
                    break;
                };
                let marker_end = marker_start + relative_end;
                if let Ok(marker) = line_ref.text[marker_start..marker_end].parse::<u16>() {
                    occurrences.push(Occurrence {
                        page: line_ref.page_index,
                        line: line_ref.line_index,
                        offset: start,
                        marker,
                        candidate: None,
                    });
                }
                cursor = marker_end + CALLOUT_END.len_utf8();
            }
            for candidate in lm2_local_ascii_callout_candidates(&line_ref.text) {
                if accepted_heads.contains(&candidate.marker) {
                    occurrences.push(Occurrence {
                        page: line_ref.page_index,
                        line: line_ref.line_index,
                        offset: candidate.start,
                        marker: candidate.marker,
                        candidate: Some((source_position, line_position, candidate.leading)),
                    });
                }
            }
        }
    }
    occurrences.sort_by_key(|item| (item.page, item.line, item.offset));

    let mut operations = Vec::<(usize, usize, usize, usize, u16)>::new();
    for (position, occurrence) in occurrences.iter().enumerate() {
        let Some((source_position, line_position, leading)) = occurrence.candidate else {
            continue;
        };
        let previous = occurrences[..position]
            .iter()
            .rev()
            .find(|item| item.candidate.is_none());
        let next = occurrences[position + 1..]
            .iter()
            .find(|item| item.candidate.is_none());
        if previous.map(|item| item.marker) != Some(occurrence.marker.saturating_sub(1))
            || next.map(|item| item.marker) != Some(occurrence.marker.saturating_add(1))
        {
            continue;
        }
        if leading {
            let current_ref = &sources[source_position].lines[line_position];
            let Some(current) = current_ref
                .id
                .as_deref()
                .and_then(|id| line_by_id.get(id).copied())
            else {
                continue;
            };
            let preceding = sources
                .iter()
                .flat_map(|source| source.lines.iter())
                .filter(|line| {
                    line.page_index == current_ref.page_index
                        && line.line_index < current_ref.line_index
                })
                .max_by_key(|line| line.line_index)
                .and_then(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| line_by_id.get(id).copied())
                });
            if preceding.is_none_or(|previous| {
                !same_visual_baseline_fragment(previous, current)
                    && !lm2_same_row_leading_callout_with_prose(
                        previous,
                        current,
                        occurrence.marker,
                    )
            }) {
                continue;
            }
        }
        let candidate =
            lm2_local_ascii_callout_candidates(&sources[source_position].lines[line_position].text)
                .into_iter()
                .find(|candidate| {
                    candidate.marker == occurrence.marker && candidate.start == occurrence.offset
                });
        if let Some(candidate) = candidate {
            operations.push((
                source_position,
                line_position,
                candidate.start,
                candidate.end,
                candidate.marker,
            ));
        }
    }

    let mut repaired = 0usize;
    for (source_position, line_position, start, end, marker) in operations.into_iter().rev() {
        let source = &mut sources[source_position];
        let original = source.lines[line_position].text.clone();
        let mut rewritten = original.clone();
        rewritten.replace_range(start..end, &format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
        source.lines[line_position].text = rewritten.clone();
        source.lines[line_position].role = LiquidBlockRole::Paragraph;
        source.lines[line_position].note_markers.clear();
        let promote_whole_block = source.lines.iter().all(|line| {
            matches!(
                line.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            )
        });
        if let Some(block) = blocks.get_mut(source.block_index)
            && block.text.contains(&original)
        {
            block.text = block.text.replacen(&original, &rewritten, 1);
            if promote_whole_block {
                block.role = LiquidBlockRole::Paragraph;
                block.label = None;
            }
            repaired += 1;
        }
    }
    repaired
}

/// Recover ASCII footnote callouts flattened into body text anywhere on a
/// source line, but only when the complete note-head sequence on that page can
/// be matched in reading order. The all-or-nothing page gate prevents a legal
/// section number or year from becoming a link merely because it ends in the
/// same digits as one note. A special case accepts a four-digit year followed
/// immediately by its superscript (`19891` = `1989` + note 1).
pub(super) fn apply_page_sequence_ascii_callout_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    #[derive(Clone)]
    struct BodyLine<'a> {
        block_index: usize,
        line_index: usize,
        text: &'a str,
    }

    let mut repaired = apply_sentineled_marginalia_page_zone_recovery(blocks, sources);

    // PDF text layers sometimes place two short footnotes on the same visual
    // row (`19. ... 20. ...`). The ordinary source marker records only the
    // first head, so recover the later head before both callout alignment and
    // final Markdown linking consume this provenance.
    for source in sources.iter_mut() {
        for line in &mut source.lines {
            if !matches!(
                line.role,
                LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
            ) {
                continue;
            }
            let contextual = embedded_numeric_note_head_markers(&line.text);
            if contextual.is_empty() && line.note_markers.is_empty() {
                continue;
            }
            // A page-break continuation can begin with the preceding note's
            // pincite immediately before the real, sentineled next head:
            // `630. <46> 539 U.S. ...`.  Keep the contextual head and discard
            // the wildly nonsequential bare number from provenance.
            if let Some(leading) = leading_numeric_token_marker(&line.text)
                && contextual
                    .iter()
                    .copied()
                    .any(|marker| leading > marker.saturating_add(3))
            {
                line.note_markers.retain(|marker| *marker != leading);
            }
            line.note_markers.extend(contextual);
            line.note_markers.sort_unstable();
            line.note_markers.dedup();
        }
        repaired += enrich_sequential_note_heads_within_block(source);
    }

    let mut note_markers_by_page: BTreeMap<usize, Vec<(usize, u16)>> = BTreeMap::new();
    let mut body_lines_by_page: BTreeMap<usize, Vec<BodyLine<'_>>> = BTreeMap::new();
    for source in sources {
        for line in &source.lines {
            if matches!(
                line.role,
                LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
            ) {
                for marker in &line.note_markers {
                    note_markers_by_page
                        .entry(line.page_index)
                        .or_default()
                        .push((line.line_index, *marker));
                }
            } else if matches!(
                line.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
            ) {
                body_lines_by_page
                    .entry(line.page_index)
                    .or_default()
                    .push(BodyLine {
                        block_index: source.block_index,
                        line_index: line.line_index,
                        text: &line.text,
                    });
            }
        }
    }

    for (page_index, marker_rows) in note_markers_by_page {
        let mut marker_rows = marker_rows;
        marker_rows.sort_unstable();
        let mut markers = Vec::new();
        for (_, marker) in marker_rows {
            if markers.last() != Some(&marker) {
                markers.push(marker);
            }
        }
        if markers.is_empty()
            || (markers.len() >= 2
                && !markers
                    .windows(2)
                    .all(|pair| pair[1] > pair[0] && pair[1] <= pair[0].saturating_add(3)))
        {
            continue;
        }
        let Some(mut body_lines) = body_lines_by_page.remove(&page_index) else {
            continue;
        };
        body_lines.sort_by_key(|line| line.line_index);

        let singleton_page = markers.len() == 1;
        let mut matches = Vec::<(usize, usize, usize, usize, u16, PageSequenceMatchKind)>::new();
        let mut cleanup_lines = Vec::<(usize, String)>::new();
        let mut body_position = 0usize;
        let mut byte_position = 0usize;
        let mut complete = true;
        for marker in markers {
            let mut found: Option<(usize, usize, usize, usize, PageSequenceMatchKind)> = None;
            for matched_body_position in body_position..body_lines.len() {
                let line = &body_lines[matched_body_position];
                let from = if matched_body_position == body_position {
                    byte_position
                } else {
                    0
                };
                if let Some((start, end, kind)) =
                    page_sequence_callout_range(line.text, marker, from)
                {
                    if singleton_page && !kind.singleton_safe() {
                        continue;
                    }
                    let candidate = (matched_body_position, line.block_index, start, end, kind);
                    if found.as_ref().is_none_or(
                        |(best_position, _, best_start, best_end, best_kind)| {
                            (
                                sequence_selection_priority(kind, line.text, start, end),
                                matched_body_position,
                                start,
                            ) < (
                                sequence_selection_priority(
                                    *best_kind,
                                    body_lines[*best_position].text,
                                    *best_start,
                                    *best_end,
                                ),
                                *best_position,
                                *best_start,
                            )
                        },
                    ) {
                        found = Some(candidate);
                    }
                }
            }
            let Some((line_position, block_index, start, end, kind)) = found else {
                complete = false;
                break;
            };
            body_position = line_position;
            if kind == PageSequenceMatchKind::InferredQuoteBoundary {
                body_position += 1;
                byte_position = 0;
            } else {
                byte_position = end;
            }
            if kind.replaces_text() {
                if kind == PageSequenceMatchKind::PartialDigit
                    && let Some(next) = body_lines.get(line_position + 1)
                    && next.line_index == body_lines[line_position].line_index.saturating_add(1)
                    && split_callout_fragment_completes_marker(
                        &body_lines[line_position].text[start..end],
                        next.text,
                        marker,
                    )
                {
                    cleanup_lines.push((next.block_index, clean_lm2_line_text(next.text)));
                }
                matches.push((block_index, line_position, start, end, marker, kind));
            }
        }
        if !complete || matches.is_empty() {
            continue;
        }

        // Rebuild each affected source line, then replace that exact line once
        // inside its assembled block. This keeps offsets local even when many
        // source lines were joined into one paragraph block.
        let mut by_line = BTreeMap::<(usize, usize), Vec<(usize, usize, u16)>>::new();
        for (block_index, line_position, start, end, marker, _) in matches {
            by_line
                .entry((block_index, line_position))
                .or_default()
                .push((start, end, marker));
        }
        for ((block_index, line_position), mut ranges) in by_line {
            let Some(line) = body_lines.get(line_position) else {
                continue;
            };
            let clean = clean_lm2_line_text(line.text);
            if clean != line.text {
                // Source cleaning can change byte offsets; leave this rare line
                // to the existing terminal recovery rather than guessing.
                continue;
            }
            ranges.sort_by_key(|(start, _, _)| *start);
            let mut rewritten = clean.clone();
            for (start, end, marker) in ranges.into_iter().rev() {
                rewritten
                    .replace_range(start..end, &format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
            }
            let Some(block) = blocks.get_mut(block_index) else {
                continue;
            };
            if block.text.contains(&clean) {
                block.text = block.text.replacen(&clean, &rewritten, 1);
                repaired += 1;
            }
        }
        for (block_index, fragment) in cleanup_lines {
            let Some(block) = blocks.get_mut(block_index) else {
                continue;
            };
            remove_standalone_source_fragment(&mut block.text, &fragment);
        }
    }
    repaired
}

/// Some journals place several superscript note heads on the same physical
/// line and omit a divider that the geometric detector can trust. Once a
/// source-Marginalia line contains a consecutive pair of sentineled heads, the
/// rest of that page is a note zone. Restore body/table fragments in that
/// trailing zone to Marginalia and retain every provenance-backed head.
pub(super) fn apply_sentineled_marginalia_page_zone_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    let mut candidates = BTreeMap::<usize, Vec<(usize, u16)>>::new();
    for source in sources.iter() {
        for line in &source.lines {
            if line.role != LiquidBlockRole::Marginalia {
                continue;
            }
            for marker in sentineled_note_head_markers_in_context(&line.text) {
                candidates
                    .entry(line.page_index)
                    .or_default()
                    .push((line.line_index, marker));
            }
        }
    }
    let zone_start = candidates
        .into_iter()
        .filter_map(|(page_index, mut heads)| {
            heads.sort_unstable();
            heads.dedup();
            let consecutive = heads
                .windows(2)
                .any(|pair| pair[1].1 == pair[0].1.saturating_add(1));
            consecutive.then_some((page_index, heads[0].0))
        })
        .collect::<BTreeMap<_, _>>();
    if zone_start.is_empty() {
        return 0;
    }

    let mut repaired = 0usize;
    for source in sources.iter_mut() {
        for line in &mut source.lines {
            let Some(start) = zone_start.get(&line.page_index).copied() else {
                continue;
            };
            if line.role == LiquidBlockRole::Marginalia && line.line_index >= start {
                line.note_markers
                    .extend(sentineled_note_head_markers_in_context(&line.text));
                line.note_markers.sort_unstable();
                line.note_markers.dedup();
            }
        }
        let Some(first) = source.lines.first() else {
            continue;
        };
        let Some(start) = zone_start.get(&first.page_index).copied() else {
            continue;
        };
        if first.line_index < start
            || source
                .lines
                .iter()
                .any(|line| line.page_index != first.page_index)
        {
            continue;
        }
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        let provenance_backed_noise = block.role == LiquidBlockRole::Noise
            && source
                .lines
                .iter()
                .all(|line| line.role == LiquidBlockRole::Marginalia)
            && source
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty());
        if matches!(
            block.role,
            LiquidBlockRole::Title
                | LiquidBlockRole::AuthorInfo
                | LiquidBlockRole::Heading
                | LiquidBlockRole::Subheading
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
        ) || block.role == LiquidBlockRole::Noise && !provenance_backed_noise
        {
            continue;
        }
        if block.role != LiquidBlockRole::Marginalia {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
            repaired += 1;
        }
        for line in &mut source.lines {
            line.role = LiquidBlockRole::Marginalia;
            line.note_markers
                .extend(sentineled_note_head_markers_in_context(&line.text));
            line.note_markers.sort_unstable();
            line.note_markers.dedup();
        }
    }
    repaired
}

/// A PDF text block can contain the end of one footnote and the complete next
/// note. The upstream detector marks only the block's first head. Recover the
/// later head when it is exactly sequential, nearby on the same page, and
/// follows a sentence-ending continuation line.
pub(super) fn enrich_sequential_note_heads_within_block(
    source: &mut LiquidBlockSourceLines,
) -> usize {
    let mut repaired = 0usize;
    let mut previous_marker: Option<(usize, usize, u16)> = None;
    let mut previous_text = String::new();

    for line in &mut source.lines {
        if !matches!(
            line.role,
            LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
        ) {
            previous_marker = None;
            previous_text.clear();
            continue;
        }

        if let Some(marker) = line.note_markers.iter().copied().max() {
            previous_marker = Some((line.page_index, line.line_index, marker));
        } else if let Some((page_index, marker_line_index, marker)) = previous_marker
            && line.page_index == page_index
            && line.line_index <= marker_line_index.saturating_add(8)
            && previous_text
                .trim_end()
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ')' | ']'))
            && let Some(candidate) = lm2_leading_numeric_or_bare_marker(&line.text)
            && candidate == marker.saturating_add(1)
        {
            line.note_markers.push(candidate);
            previous_marker = Some((line.page_index, line.line_index, candidate));
            repaired += 1;
        }
        previous_text.clone_from(&line.text);
    }

    repaired
}

/// Later numbered heads embedded in a marginalia row. Requiring both the
/// marker's own punctuation and sentence punctuation before an embedded head
/// avoids treating reporter volumes or section numbers as new notes.
pub(super) fn embedded_numeric_note_head_markers(text: &str) -> Vec<u16> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut markers = Vec::new();
    let mut position = 0usize;
    while position < chars.len() {
        if !chars[position].1.is_ascii_digit()
            || position > 0 && !chars[position - 1].1.is_whitespace()
        {
            position += 1;
            continue;
        }
        let start_position = position;
        let start = chars[position].0;
        while position < chars.len()
            && chars[position].1.is_ascii_digit()
            && position - start_position < 3
        {
            position += 1;
        }
        let end = chars
            .get(position)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        let Some(punctuation) = chars.get(position).map(|(_, ch)| *ch) else {
            continue;
        };
        if !matches!(punctuation, '.' | ')' | ']')
            || chars
                .get(position + 1)
                .is_some_and(|(_, ch)| !ch.is_whitespace())
        {
            continue;
        }
        let at_start = text[..start].trim().is_empty();
        let after_punctuation = chars
            .get(position + 1)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        if !at_start && text[after_punctuation..].trim().is_empty() {
            // A terminal reporter, clause, or docket number (`... cl. 2.` or
            // `Dkt. No. 176.`) is not a later note head.
            continue;
        }
        let embedded_after_sentence = text[..start]
            .trim_end()
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ')' | ']'));
        if !(at_start || embedded_after_sentence) {
            continue;
        }
        if let Ok(marker) = text[start..end].parse::<u16>()
            && (1..=LM2_MAX_NOTE_MARKER).contains(&marker)
        {
            markers.push(marker);
        }
    }
    markers.extend(sentineled_note_head_markers_in_context(text));
    markers.sort_unstable();
    markers.dedup();
    markers
}

pub(super) fn sentineled_note_head_markers_in_context(text: &str) -> Vec<u16> {
    let mut markers = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = text[cursor..].find(CALLOUT_START) {
        let start = cursor + relative_start;
        let tail_start = start + CALLOUT_START.len_utf8();
        let Some(relative_end) = text[tail_start..].find(CALLOUT_END) else {
            break;
        };
        let end = tail_start + relative_end;
        let after_end = end + CALLOUT_END.len_utf8();
        let marker = text[tail_start..end].parse::<u16>().ok();
        let before_is_boundary = text[..start].trim().is_empty()
            || text[..start]
                .trim_end()
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ')' | ']'));
        let after_is_space = text[after_end..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        if before_is_boundary
            && after_is_space
            && let Some(marker) = marker
            && (1..=LM2_MAX_NOTE_MARKER).contains(&marker)
        {
            markers.push(marker);
        }
        cursor = after_end;
    }
    markers
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PageSequenceMatchKind {
    Existing,
    SplitExisting,
    ExactAscii,
    OcrDigits,
    PartialExisting,
    PartialDigit,
    Placeholder,
    InferredQuoteBoundary,
}

impl PageSequenceMatchKind {
    pub(super) fn replaces_text(self) -> bool {
        self != Self::Existing
    }

    pub(super) fn singleton_safe(self) -> bool {
        matches!(
            self,
            Self::Existing | Self::SplitExisting | Self::ExactAscii | Self::OcrDigits
        )
    }
}

/// `(start, end, replace)` for an existing sentinel or an attached ASCII
/// callout. Existing sentinels advance sequence matching but need no rewrite.
pub(super) fn page_sequence_callout_range(
    text: &str,
    marker: u16,
    from: usize,
) -> Option<(usize, usize, PageSequenceMatchKind)> {
    let digits = marker.to_string();
    let sentinel = format!("{CALLOUT_START}{marker}{CALLOUT_END}");
    let cursor = from.min(text.len());
    let sentinel_match = text[cursor..].find(&sentinel).map(|offset| {
        (
            cursor + offset,
            sentinel.len(),
            PageSequenceMatchKind::Existing,
        )
    });
    let split_existing_match = split_existing_sentinel_range(text, &digits, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::SplitExisting));
    let ascii_match = attached_page_sequence_ascii_range(text, &digits, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::ExactAscii));
    let ocr_match = ocr_digit_sequence_range(text, &digits, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::OcrDigits));
    let partial_existing_match = partial_existing_sentinel_range(text, &digits, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::PartialExisting));
    let partial_match = partial_page_sequence_digit_range(text, &digits, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::PartialDigit));
    let placeholder_match = page_sequence_placeholder_range(text, cursor)
        .map(|(start, end)| (start, end - start, PageSequenceMatchKind::Placeholder));
    let inferred_match = inferred_quote_boundary_range(text, cursor)
        .map(|position| (position, 0, PageSequenceMatchKind::InferredQuoteBoundary));

    let mut candidates = [
        sentinel_match,
        split_existing_match,
        ascii_match,
        ocr_match,
        partial_existing_match,
        partial_match,
        placeholder_match,
        inferred_match,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    candidates.sort_by_key(|(start, _, kind)| (match_priority(*kind), *start));
    let (start, len, kind) = candidates.into_iter().next()?;
    Some((start, start + len, kind))
}

pub(super) fn match_priority(kind: PageSequenceMatchKind) -> u8 {
    match kind {
        PageSequenceMatchKind::Existing => 0,
        PageSequenceMatchKind::SplitExisting => 1,
        PageSequenceMatchKind::ExactAscii => 2,
        PageSequenceMatchKind::OcrDigits => 3,
        PageSequenceMatchKind::PartialExisting => 4,
        PageSequenceMatchKind::PartialDigit => 5,
        PageSequenceMatchKind::Placeholder => 6,
        PageSequenceMatchKind::InferredQuoteBoundary => 7,
    }
}

/// Prefer explicit sentinels and demote only the two genuinely weak
/// fallbacks: an alphabetic OCR substitution (`s` -> `8`) and a wholly
/// inferred quote-boundary marker. Other candidates retain reading-order
/// priority, so a later legal number cannot displace an earlier flattened
/// callout.
pub(super) fn sequence_selection_priority(
    kind: PageSequenceMatchKind,
    text: &str,
    start: usize,
    end: usize,
) -> u8 {
    match kind {
        PageSequenceMatchKind::Existing | PageSequenceMatchKind::SplitExisting => 0,
        PageSequenceMatchKind::OcrDigits
            if text
                .get(start..end)
                .is_some_and(|value| value.chars().any(char::is_alphabetic)) =>
        {
            2
        }
        PageSequenceMatchKind::InferredQuoteBoundary => 2,
        _ => 1,
    }
}

/// Match a multi-digit callout that an earlier permissive pass encoded as one
/// sentinel per glyph (`26` -> `[^2] [^6]`). The complete page-level note
/// sequence is the confidence gate for collapsing the fragments.
pub(super) fn split_existing_sentinel_range(
    text: &str,
    digits: &str,
    from: usize,
) -> Option<(usize, usize)> {
    if digits.len() < 2 {
        return None;
    }
    let fragments = digits
        .chars()
        .map(|digit| format!("{CALLOUT_START}{digit}{CALLOUT_END}"))
        .collect::<Vec<_>>();
    let first = fragments.first()?;
    let mut cursor = from.min(text.len());
    while let Some(offset) = text.get(cursor..)?.find(first) {
        let start = cursor + offset;
        let mut end = start + first.len();
        let mut complete = true;
        for fragment in fragments.iter().skip(1) {
            let whitespace = text[end..]
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .map(char::len_utf8)
                .sum::<usize>();
            end += whitespace;
            if !text[end..].starts_with(fragment) {
                complete = false;
                break;
            }
            end += fragment.len();
        }
        if complete {
            return Some((start, end));
        }
        cursor = start + first.len();
    }
    None
}

/// Match the one surviving digit of a multi-digit callout after that glyph was
/// already encoded as a one-digit sentinel by a permissive local pass.
pub(super) fn partial_existing_sentinel_range(
    text: &str,
    digits: &str,
    from: usize,
) -> Option<(usize, usize)> {
    if digits.len() != 2 {
        return None;
    }
    digits
        .chars()
        .filter_map(|digit| {
            let sentinel = format!("{CALLOUT_START}{digit}{CALLOUT_END}");
            text.get(from.min(text.len())..)?
                .find(&sentinel)
                .map(|offset| {
                    let start = from.min(text.len()) + offset;
                    (start, start + sentinel.len())
                })
                .filter(|(start, end)| plausible_partial_existing_callout(text, *start, *end))
        })
        .min_by_key(|(start, _)| *start)
}

pub(super) fn plausible_partial_existing_callout(text: &str, start: usize, end: usize) -> bool {
    let before = &text[..start];
    let immediately_before = before.chars().next_back();
    let before_that = immediately_before.and_then(|ch| {
        before[..before.len().saturating_sub(ch.len_utf8())]
            .chars()
            .next_back()
    });
    if immediately_before == Some('.') && before_that.is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }

    let next_visible = text[end..].chars().find(|ch| !ch.is_whitespace());
    if matches!(
        (immediately_before, next_visible),
        (Some('('), Some(')')) | (Some('['), Some(']'))
    ) {
        return false;
    }
    if immediately_before.is_some_and(char::is_alphabetic) {
        let preceding_word_len = before
            .chars()
            .rev()
            .take_while(|ch| ch.is_alphabetic())
            .count();
        if preceding_word_len == 1
            && next_visible.is_some_and(|ch| matches!(ch, ',' | ';' | ':' | ')' | ']'))
        {
            return false;
        }
    }
    true
}

/// Match a complete multi-digit callout even when the text layer separated its
/// glyphs (`26` -> `2 6`) or confused a superscript glyph (`28` -> `2s`).
pub(super) fn ocr_digit_sequence_range(
    text: &str,
    digits: &str,
    from: usize,
) -> Option<(usize, usize)> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for start_position in 0..chars.len() {
        let start = chars[start_position].0;
        if start < from || !ocr_digit_matches(chars[start_position].1, digits.as_bytes()[0]) {
            continue;
        }
        if !callout_left_boundary(text, start) {
            continue;
        }
        let mut position = start_position;
        let mut matched = true;
        for (digit_index, digit) in digits.as_bytes().iter().enumerate() {
            if digit_index > 0 {
                while position < chars.len() && chars[position].1.is_whitespace() {
                    position += 1;
                }
            }
            if position >= chars.len() || !ocr_digit_matches(chars[position].1, *digit) {
                matched = false;
                break;
            }
            position += 1;
        }
        if !matched {
            continue;
        }
        let end = chars
            .get(position)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        if text[end..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        {
            continue;
        }
        return Some((start, end));
    }
    None
}

pub(super) fn ocr_digit_matches(ch: char, digit: u8) -> bool {
    ch as u32 == digit as u32
        || matches!(
            (digit, ch),
            (b'0', 'o' | 'O') | (b'1', 'l' | 'I' | '|' | '\'') | (b'8', 's' | 'S')
        )
}

/// Match the one surviving glyph of a two-digit superscript. This is admitted
/// only inside a complete local note-head sequence.
pub(super) fn partial_page_sequence_digit_range(
    text: &str,
    digits: &str,
    from: usize,
) -> Option<(usize, usize)> {
    if digits.len() != 2 {
        return None;
    }
    for (start, ch) in text.char_indices() {
        if start < from || !ch.is_ascii_digit() {
            continue;
        }
        if ch as u8 != digits.as_bytes()[0] && ch as u8 != digits.as_bytes()[1] {
            continue;
        }
        let end = start + ch.len_utf8();
        if !callout_left_boundary(text, start)
            || text[end..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_alphanumeric())
        {
            continue;
        }
        return Some((start, end));
    }
    None
}

pub(super) fn callout_left_boundary(text: &str, start: usize) -> bool {
    let left = &text[..start];
    let immediate = left.chars().next_back();
    if immediate.is_some_and(|ch| !ch.is_whitespace()) {
        return immediate.is_some_and(|ch| {
            !ch.is_ascii_alphanumeric()
                && !matches!(ch, '\u{00A7}' | '-' | '*' | CALLOUT_START | CALLOUT_END)
        });
    }
    left.trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ':' | ';' | ')' | ']' | '"' | '\''))
}

/// A lone apostrophe after sentence or quotation punctuation is a recurring
/// superscript-font extraction artifact in older law-review PDFs.
pub(super) fn page_sequence_placeholder_range(text: &str, from: usize) -> Option<(usize, usize)> {
    for (start, ch) in text.char_indices() {
        if start < from || ch != '\'' {
            continue;
        }
        let before = text[..start].chars().next_back();
        let after = text[start + ch.len_utf8()..].chars().next();
        if before.is_some_and(|ch| matches!(ch, '.' | '?' | '!' | '"'))
            && after.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
        {
            return Some((start, start + ch.len_utf8()));
        }
    }
    None
}

/// Last-resort recovery for a callout whose glyph vanished completely after a
/// sentence-ending quotation. The page-level all-or-nothing sequence is the
/// confidence gate; ordinary quoted prose on singleton pages cannot enter.
pub(super) fn inferred_quote_boundary_range(text: &str, from: usize) -> Option<usize> {
    let trimmed = text.trim_end();
    let end = trimmed.len();
    (end >= from
        && (trimmed.ends_with(".\"") || trimmed.ends_with("?\"") || trimmed.ends_with("!\"")))
    .then_some(end)
}

pub(super) fn split_callout_fragment_completes_marker(
    current: &str,
    next: &str,
    marker: u16,
) -> bool {
    let current = current.trim();
    let next = next.trim();
    current.len() == 1
        && next.len() == 1
        && current.chars().all(|ch| ch.is_ascii_digit())
        && next.chars().all(|ch| ch.is_ascii_digit())
        && format!("{current}{next}") == marker.to_string()
}

pub(super) fn remove_standalone_source_fragment(block_text: &mut String, fragment: &str) {
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return;
    }
    let trimmed = block_text.trim_start();
    if let Some(remainder) = trimmed.strip_prefix(fragment)
        && remainder.chars().next().is_none_or(|ch| ch.is_whitespace())
    {
        *block_text = remainder.trim_start().to_owned();
    }
}

pub(super) fn attached_page_sequence_ascii_range(
    text: &str,
    digits: &str,
    from: usize,
) -> Option<(usize, usize)> {
    let mut cursor = from.min(text.len());
    while let Some(offset) = text.get(cursor..)?.find(digits) {
        let start = cursor + offset;
        let end = start + digits.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let mut accepted = before.is_some_and(|before| {
            !before.is_whitespace()
                && !matches!(before, '\u{00A7}' | '-' | '*' | CALLOUT_START | CALLOUT_END)
                && !after.is_some_and(|ch| ch.is_ascii_digit())
        });
        if accepted && before.is_some_and(|ch| ch.is_ascii_digit()) {
            let prefix_digits = text[..start]
                .chars()
                .rev()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            let plausible_year = prefix_digits.len() == 4
                && prefix_digits
                    .parse::<u16>()
                    .is_ok_and(|year| (1500..=2200).contains(&year));
            accepted = plausible_year;
        }
        if accepted {
            return Some((start, end));
        }
        cursor = end;
    }
    None
}

pub(super) fn apply_in_block_standalone_callout_recovery(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let block_by_line_id = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().filter_map(move |line| {
                line.id.as_ref().map(|id| (id.as_str(), source.block_index))
            })
        })
        .collect::<HashMap<_, _>>();
    // Decoder storage order is not a reading-order contract.  Superscript
    // fragments must be paired with the preceding physical source line, not
    // merely the preceding element in `decoded`.
    let line_by_coordinate = decoded
        .iter()
        .map(|(line, _)| ((line.page_index, line.line_index), line))
        .collect::<HashMap<_, _>>();
    let accepted_note_pages = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().flat_map(|line| {
                line.note_markers
                    .iter()
                    .map(move |marker| (line.page_index, *marker))
            })
        })
        .collect::<HashSet<_>>();

    let mut repaired = 0usize;
    for (marker_line, _) in decoded {
        let Some(marker) = marker_line.text.trim().parse::<u16>().ok() else {
            continue;
        };
        if !(1..=LM2_MAX_NOTE_MARKER).contains(&marker) {
            continue;
        }
        let Some(previous_line_index) = marker_line.line_index.checked_sub(1) else {
            continue;
        };
        let Some(previous) = line_by_coordinate
            .get(&(marker_line.page_index, previous_line_index))
            .copied()
        else {
            continue;
        };
        let Some(marker_block) = block_by_line_id.get(marker_line.id.as_str()) else {
            continue;
        };
        let Some(previous_block) = block_by_line_id.get(previous.id.as_str()) else {
            continue;
        };
        let strict_geometry = lm2_marker_can_attach_to_previous_line(marker_line, previous);
        // Some PDF text layers collapse two visual body rows into one source
        // line.  Its bounding box then reaches the page margin even though the
        // superscript follows the final glyph on the lower row.  For a
        // small-font numeric fragment, an accepted same-page note definition
        // plus vertical overlap is a safer contract than the collapsed line's
        // misleading `right` edge.
        let note_backed_overlap = accepted_note_pages.contains(&(marker_line.page_index, marker))
            && marker_line.page_index == previous.page_index
            && marker_line.font_height <= previous.font_height * 0.80
            && marker_line.bottom <= previous.top
            && marker_line.top >= previous.bottom
            && marker_line.left >= previous.left - 2.0
            && marker_line.left <= previous.right + 4.0;
        if !strict_geometry && !note_backed_overlap {
            continue;
        }
        if previous_block != marker_block {
            if *previous_block >= blocks.len()
                || *marker_block >= blocks.len()
                || blocks[*previous_block].role != LiquidBlockRole::Paragraph
            {
                continue;
            }
            let marker_text = marker.to_string();
            let marker_block_text = blocks[*marker_block].text.trim_start();
            let exact_marker_block = marker_block_text == marker_text;
            let leading_marker_before_note = marker_block_text
                .strip_prefix(&marker_text)
                .filter(|remainder| remainder.chars().next().is_some_and(char::is_whitespace))
                .map(str::trim_start)
                .filter(|remainder| !remainder.is_empty())
                .map(str::to_owned);
            if !exact_marker_block && leading_marker_before_note.is_none() {
                continue;
            }
            blocks[*previous_block].text = format!(
                "{}{}{}{}",
                blocks[*previous_block].text.trim_end(),
                CALLOUT_START,
                marker,
                CALLOUT_END
            );
            if exact_marker_block {
                blocks[*marker_block].role = LiquidBlockRole::Noise;
                blocks[*marker_block].label = None;
            } else if let Some(remainder) = leading_marker_before_note {
                blocks[*marker_block].text = remainder;
            }
            repaired += 1;
            continue;
        }
        let Some(block) = blocks.get_mut(*marker_block) else {
            continue;
        };
        let plain = format!(" {marker}");
        let Some(position) = block.text.rfind(&plain) else {
            continue;
        };
        let replacement = format!("{CALLOUT_START}{marker}{CALLOUT_END}");
        block
            .text
            .replace_range(position..position + plain.len(), &replacement);
        repaired += 1;
    }
    repaired
}

/// Some text layers combine a superscript with the first words after it and
/// emit that fragment as the next physical line (`346 At that`). When the
/// fragment shares the preceding line's baseline, move the marker backward and
/// rejoin the prose.
pub(super) fn apply_same_row_leading_callout_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if blocks.len() < 2 {
        return 0;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let note_pages = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().flat_map(|line| {
                line.note_markers
                    .iter()
                    .map(move |marker| (line.page_index, *marker))
            })
        })
        .collect::<HashSet<_>>();
    let mut repaired = 0usize;
    for source in sources.iter() {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        let ordinary_body_role = matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        );
        let unmarked_body_candidate =
            matches!(
                block.role,
                LiquidBlockRole::Noise | LiquidBlockRole::Marginalia
            ) && source.lines.iter().all(|line| line.note_markers.is_empty());
        if !ordinary_body_role && !unmarked_body_candidate {
            continue;
        }
        for pair in source.lines.windows(2) {
            let (previous_ref, current_ref) = (&pair[0], &pair[1]);
            let (Some(previous_id), Some(current_id)) =
                (previous_ref.id.as_deref(), current_ref.id.as_deref())
            else {
                continue;
            };
            let (Some(previous_line), Some(current_line)) =
                (line_by_id.get(previous_id), line_by_id.get(current_id))
            else {
                continue;
            };
            let Some(marker) = leading_numeric_token_marker(&current_ref.text)
                .or_else(|| leading_sentineled_marker(&current_ref.text))
            else {
                continue;
            };
            if !lm2_same_row_leading_callout(previous_line, current_line)
                && !(note_pages.contains(&(current_ref.page_index, marker))
                    && lm2_same_row_leading_callout_with_prose(previous_line, current_line, marker))
            {
                continue;
            }
            let clean = clean_lm2_line_text(&current_ref.text);
            let Some(remainder) = lm2_strip_leading_ascii_or_decoded_callout(&clean, marker) else {
                continue;
            };
            if let Some(position) = block.text.find(&clean) {
                let replacement = format!("{CALLOUT_START}{marker}{CALLOUT_END} {remainder}");
                let mut start = position;
                while start > 0
                    && block.text[..start]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)
                {
                    start -= block.text[..start]
                        .chars()
                        .next_back()
                        .map(char::len_utf8)
                        .unwrap_or(0);
                }
                block
                    .text
                    .replace_range(start..position + clean.len(), &replacement);
                if unmarked_body_candidate {
                    block.role = LiquidBlockRole::Paragraph;
                    block.label = None;
                }
                repaired += 1;
            }
        }
    }
    let mut source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut index = 0usize;

    while index < old_blocks.len() {
        let can_try = index + 1 < old_blocks.len()
            && (matches!(
                old_blocks[index].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) || matches!(
                old_blocks[index].role,
                LiquidBlockRole::Heading | LiquidBlockRole::Subheading
            ) && lm2_numbered_run_in_body(&old_blocks[index].text).is_some())
            && matches!(
                old_blocks[index + 1].role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Noise | LiquidBlockRole::Marginalia
            );
        let mut merged = None;
        if can_try {
            let previous_refs = source_by_block.get(&index).cloned().unwrap_or_default();
            let current_refs = source_by_block
                .get(&(index + 1))
                .cloned()
                .unwrap_or_default();
            if let (Some(previous_ref), Some(current_ref)) =
                (previous_refs.last(), current_refs.first())
                && current_refs.iter().all(|line| line.note_markers.is_empty())
                && let (Some(previous_id), Some(current_id)) =
                    (previous_ref.id.as_deref(), current_ref.id.as_deref())
                && let (Some(previous_line), Some(current_line)) =
                    (line_by_id.get(previous_id), line_by_id.get(current_id))
                && let Some(marker) = leading_numeric_token_marker(&current_ref.text)
                    .or_else(|| leading_sentineled_marker(&current_ref.text))
                && (lm2_same_row_leading_callout(previous_line, current_line)
                    || (note_pages.contains(&(current_ref.page_index, marker))
                        && lm2_same_row_leading_callout_with_prose(
                            previous_line,
                            current_line,
                            marker,
                        )))
                && let Some(remainder) =
                    lm2_strip_leading_ascii_or_decoded_callout(&old_blocks[index + 1].text, marker)
            {
                let mut block = old_blocks[index].clone();
                block.text = format!(
                    "{}{}{}{}",
                    block.text.trim_end(),
                    CALLOUT_START,
                    marker,
                    CALLOUT_END
                );
                append_line(&mut block.text, remainder);
                let mut refs = previous_refs.clone();
                refs.extend(current_refs.clone());
                merged = Some((block, refs));
            }
        }

        // A same-baseline callout can already have been moved into the
        // preceding Paragraph while the following physical body row remains a
        // separate Paragraph. Join that one adjacent row only when the prior
        // source ends in a Noise callout fragment, the accepted note exists on
        // the page, and the next row returns to the base row's body margin.
        if merged.is_none()
            && index + 1 < old_blocks.len()
            && old_blocks[index].role == LiquidBlockRole::Paragraph
            && old_blocks[index + 1].role == LiquidBlockRole::Paragraph
        {
            let previous_refs = source_by_block.get(&index).cloned().unwrap_or_default();
            let current_refs = source_by_block
                .get(&(index + 1))
                .cloned()
                .unwrap_or_default();
            if previous_refs.len() >= 2
                && !current_refs.is_empty()
                && current_refs.iter().all(|line| line.note_markers.is_empty())
            {
                let base_ref = &previous_refs[previous_refs.len() - 2];
                let marker_ref = &previous_refs[previous_refs.len() - 1];
                let current_ref = &current_refs[0];
                if marker_ref.role == LiquidBlockRole::Noise
                    && marker_ref.note_markers.is_empty()
                    && let Some(marker) = leading_numeric_token_marker(&marker_ref.text)
                        .or_else(|| leading_sentineled_marker(&marker_ref.text))
                    && note_pages.contains(&(marker_ref.page_index, marker))
                    && old_blocks[index]
                        .text
                        .contains(&format!("{CALLOUT_START}{marker}{CALLOUT_END}"))
                    && lm2_reflow_paragraph_is_visibly_open(&old_blocks[index].text)
                    && lm2_reflow_starts_like_continuation(&old_blocks[index + 1].text)
                    && let (Some(base_id), Some(marker_id), Some(current_id)) = (
                        base_ref.id.as_deref(),
                        marker_ref.id.as_deref(),
                        current_ref.id.as_deref(),
                    )
                    && let (Some(base_line), Some(marker_line), Some(current_line)) = (
                        line_by_id.get(base_id).copied(),
                        line_by_id.get(marker_id).copied(),
                        line_by_id.get(current_id).copied(),
                    )
                    && base_line.page_index == marker_line.page_index
                    && marker_line.page_index == current_line.page_index
                    && base_line.line_index.checked_add(1) == Some(marker_line.line_index)
                    && marker_line.line_index.checked_add(1) == Some(current_line.line_index)
                    && lm2_same_row_leading_callout_with_prose(base_line, marker_line, marker)
                    && lm2_body_sized_reflow_line(current_line)
                    && !current_line.doc_repeated_edge_text
                    && (current_line.left - base_line.left).abs()
                        <= (current_line.page_width.max(base_line.page_width) * 0.008).max(2.5)
                {
                    let mut block = old_blocks[index].clone();
                    append_line(&mut block.text, &old_blocks[index + 1].text);
                    let mut refs = previous_refs;
                    if let Some(marker_ref) = refs.last_mut() {
                        // The source row is now proven to be inline body prose,
                        // not standalone furniture. Preserve that conclusion
                        // through the late source-backed paragraph splitter.
                        marker_ref.role = LiquidBlockRole::Paragraph;
                    }
                    refs.extend(current_refs);
                    merged = Some((block, refs));
                }
            }
        }

        let block_index = rebuilt_blocks.len();
        if let Some((block, refs)) = merged {
            rebuilt_blocks.push(block);
            source_by_block.remove(&index);
            source_by_block.remove(&(index + 1));
            if !refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: refs,
                });
            }
            repaired += 1;
            index += 2;
        } else {
            rebuilt_blocks.push(old_blocks[index].clone());
            if let Some(lines) = source_by_block.remove(&index)
                && !lines.is_empty()
            {
                rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
            }
            index += 1;
        }
    }

    *blocks = rebuilt_blocks;
    *sources = rebuilt_sources;
    repaired
}

pub(super) fn lm2_same_row_leading_callout(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != current.page_index
        || previous.line_index.checked_add(1) != Some(current.line_index)
        || lm2_low_footnote_zone_evidence(current)
        || current.font_height > previous.font_height * 0.96
    {
        return false;
    }
    let previous_center = (previous.top + previous.bottom) * 0.5;
    let current_center = (current.top + current.bottom) * 0.5;
    let vertically_aligned = current.bottom <= previous.top
        && current.top >= previous.bottom
        && (current_center - previous_center).abs() <= previous.font_height * 0.45;
    let horizontal_tolerance = (previous.page_width.max(current.page_width) * 0.008).max(2.5);
    let horizontally_contiguous = current.left >= previous.right - 2.0
        && current.left <= previous.right + horizontal_tolerance;
    vertically_aligned && horizontally_contiguous
}

/// A superscript extracted together with the following body word inherits the
/// body's font height, so the ordinary small-fragment test cannot recognize
/// it.  Require exact same-row attachment plus a same-page accepted definition
/// before treating `153 However,` as callout 153 followed by prose.
pub(super) fn lm2_same_row_leading_callout_with_prose(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
    marker: u16,
) -> bool {
    if previous.page_index != current.page_index
        || previous.line_index.checked_add(1) != Some(current.line_index)
    {
        return false;
    }
    let Some(remainder) = lm2_strip_leading_ascii_or_decoded_callout(&current.text, marker) else {
        return false;
    };
    if !remainder
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .is_some_and(char::is_alphabetic)
    {
        return false;
    }
    let previous_center = (previous.top + previous.bottom) * 0.5;
    let current_center = (current.top + current.bottom) * 0.5;
    let vertically_aligned = (current_center - previous_center).abs()
        <= previous.font_height.max(current.font_height) * 0.35;
    let horizontal_tolerance = (previous.page_width.max(current.page_width) * 0.008).max(2.5);
    let horizontally_contiguous = current.left >= previous.right - 2.0
        && current.left <= previous.right + horizontal_tolerance;
    let previous_bbox_height = (previous.top - previous.bottom).max(0.0);
    let collapsed_multiline_tail = previous_bbox_height >= previous.font_height * 1.8
        && current.font_height <= previous.font_height * 0.95
        && current.bottom <= previous.top
        && current.top >= previous.bottom
        && current.left >= previous.left - 2.0
        && (current.right - previous.right).abs() <= horizontal_tolerance;
    let previous_can_host_marker = previous.text.trim_end().chars().last().is_some_and(|ch| {
        ch.is_ascii_alphabetic()
            || matches!(
                ch,
                '.' | '?'
                    | '!'
                    | ','
                    | ';'
                    | ':'
                    | '"'
                    | '\''
                    | '\u{2019}'
                    | '\u{201d}'
                    | ')'
                    | ']'
            )
    });
    (vertically_aligned && horizontally_contiguous || collapsed_multiline_tail)
        && previous_can_host_marker
}

/// Learned footnote state can briefly extend upward into body content on pages
/// with dense notes or tables. Require the line to be physically in the lower
/// half as well before using that state to veto body reflow and callout repair.
pub(super) fn lm2_low_footnote_zone_evidence(line: &DeepLiquidSourceLine) -> bool {
    let physically_low = line.page_height > 0.0 && line.top <= line.page_height * 0.50;
    if line.page_has_footnote_divider {
        line.below_footnote_divider
    } else {
        physically_low && (line.in_footnote_zone || line.doc_footnote_state)
    }
}

pub(super) fn lm2_strip_leading_ascii_or_decoded_callout(text: &str, marker: u16) -> Option<&str> {
    if let Some(end) = leading_callout_marker_end(text) {
        let decoded = text[CALLOUT_START.len_utf8()..end - CALLOUT_END.len_utf8()]
            .parse::<u16>()
            .ok()?;
        return (decoded == marker)
            .then(|| text[end..].trim_start())
            .filter(|remainder| !remainder.is_empty());
    }
    let digits = marker.to_string();
    let trimmed = text.trim_start();
    let remainder = trimmed.strip_prefix(&digits)?;
    if !remainder.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let remainder = remainder.trim_start();
    (!remainder.is_empty()).then_some(remainder)
}

/// Route blocks backed by ruled cells, repeated table columns, or figure
/// objects away from Heading/Marginalia before Markdown assembly.  This is a
/// layout decision, not a lexical one: labels such as `Evidence`, `Example`,
/// and `1.` are legitimate table cells and should neither become navigation
/// headings nor leak into the detached Notes fallback.
pub(super) fn apply_table_figure_block_role_routing(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let decoded_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut explicit_caption_line_by_page = BTreeMap::<usize, usize>::new();
    let mut table_figure_anchor_line_by_page = BTreeMap::<usize, usize>::new();
    for (line, _) in decoded {
        if lm2_explicit_numbered_table_figure_caption(&line.text) {
            explicit_caption_line_by_page
                .entry(line.page_index)
                .and_modify(|first| *first = (*first).min(line.line_index))
                .or_insert(line.line_index);
        }
        if lm2_numbered_table_figure_page_anchor(&line.text) {
            table_figure_anchor_line_by_page
                .entry(line.page_index)
                .and_modify(|first| *first = (*first).min(line.line_index))
                .or_insert(line.line_index);
        }
    }
    let short_label_count_by_page = decoded
        .iter()
        .filter(|(line, _)| {
            table_figure_anchor_line_by_page
                .get(&line.page_index)
                .is_some_and(|caption_line| line.line_index > *caption_line)
                && lm2_short_figure_table_label(&line.text)
        })
        .fold(BTreeMap::<usize, usize>::new(), |mut counts, (line, _)| {
            *counts.entry(line.page_index).or_default() += 1;
            counts
        });
    let any_short_label_count_by_page = decoded
        .iter()
        .filter(|(line, _)| lm2_short_figure_table_label(&line.text))
        .fold(BTreeMap::<usize, usize>::new(), |mut counts, (line, _)| {
            *counts.entry(line.page_index).or_default() += 1;
            counts
        });
    let existing_table_count_by_page = sources
        .iter()
        .filter_map(|source| {
            (blocks.get(source.block_index)?.role == LiquidBlockRole::Table)
                .then_some(source.lines.first()?.page_index)
        })
        .fold(BTreeMap::<usize, usize>::new(), |mut counts, page_index| {
            *counts.entry(page_index).or_default() += 1;
            counts
        });
    let mut repaired = 0usize;
    for source in sources {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if !matches!(
            block.role,
            LiquidBlockRole::Heading
                | LiquidBlockRole::Subheading
                | LiquidBlockRole::Paragraph
                | LiquidBlockRole::ListItem
                | LiquidBlockRole::Marginalia
        ) || source
            .lines
            .iter()
            .any(|line| !line.note_markers.is_empty())
        {
            continue;
        }
        let deep_lines = source
            .lines
            .iter()
            .filter_map(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| decoded_by_id.get(id).copied())
            })
            .collect::<Vec<_>>();
        let caption = lm2_explicit_numbered_table_figure_caption(&block.text);
        if caption {
            block.role = LiquidBlockRole::Caption;
            block.label = None;
            repaired += 1;
            continue;
        }
        let source_backed_outline_heading = lm2_outline_heading_role(&block.text).is_some()
            && word_count(&block.text) <= 24
            && source.lines.iter().any(|line| {
                matches!(
                    line.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                )
            });
        if source_backed_outline_heading {
            continue;
        }
        let short_figure_label_cluster = !deep_lines.is_empty()
            && deep_lines.first().is_some_and(|line| {
                let anchored_cluster = table_figure_anchor_line_by_page
                    .get(&line.page_index)
                    .is_some_and(|caption_line| {
                        line.line_index > *caption_line
                            && short_label_count_by_page
                                .get(&line.page_index)
                                .is_some_and(|count| *count >= 2)
                    });
                let continuation_cluster = existing_table_count_by_page
                    .get(&line.page_index)
                    .is_some_and(|count| *count >= 2)
                    || (existing_table_count_by_page
                        .get(&line.page_index)
                        .is_some_and(|count| *count >= 1)
                        && any_short_label_count_by_page
                            .get(&line.page_index)
                            .is_some_and(|count| *count >= 2));
                anchored_cluster || continuation_cluster
            })
            && deep_lines
                .iter()
                .all(|line| line.page_index == deep_lines[0].page_index)
            && source.lines.iter().all(|line| line.note_markers.is_empty())
            && source
                .lines
                .iter()
                .all(|line| lm2_short_figure_table_label(&line.text));
        if short_figure_label_cluster {
            block.role = LiquidBlockRole::Table;
            block.label = Some("Table/Figure".to_owned());
            repaired += 1;
            continue;
        }
        if deep_lines.is_empty()
            || deep_lines
                .iter()
                .any(|line| line.page_has_footnote_divider && line.below_footnote_divider)
        {
            continue;
        }
        let table_votes = deep_lines
            .iter()
            .filter(|line| {
                line.in_ruled_cell
                    || line.ruled_row_membership_exact
                    || line.page_object_ruled_row_membership
                    || line.page_table_column_like
                    || line.segment_block_table_like
                    || line.page_object_overlaps_image_bbox
            })
            .count();
        let hard_table = deep_lines.iter().any(|line| {
            line.in_ruled_cell
                || line.ruled_row_membership_exact
                || line.page_object_ruled_row_membership
        });
        if !hard_table && table_votes * 2 < deep_lines.len() {
            continue;
        }
        block.role = LiquidBlockRole::Table;
        block.label = (block.role == LiquidBlockRole::Table).then(|| "Table/Figure".to_owned());
        repaired += 1;
    }

    // The narrow label column of a two-column table can contain a lowercase
    // connector (`and Motions`) that fails the ordinary title-case cell test.
    // Exact source-line adjacency between two already-established table cells
    // is stronger evidence and keeps that one label out of body prose.
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<HashMap<_, _>>();
    let sandwiched = (1..blocks.len().saturating_sub(1))
        .filter(|index| {
            if blocks[*index - 1].role != LiquidBlockRole::Table
                || blocks[*index + 1].role != LiquidBlockRole::Table
                || !matches!(
                    blocks[*index].role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                )
                || word_count(&blocks[*index].text) > 6
            {
                return false;
            }
            let (Some(previous), Some(current), Some(next)) = (
                source_by_block.get(&(*index - 1)).copied(),
                source_by_block.get(index).copied(),
                source_by_block.get(&(*index + 1)).copied(),
            ) else {
                return false;
            };
            if current
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty())
            {
                return false;
            }
            let (Some(previous_line), Some(current_first), Some(current_last), Some(next_line)) = (
                previous.lines.last(),
                current.lines.first(),
                current.lines.last(),
                next.lines.first(),
            ) else {
                return false;
            };
            previous_line.page_index == current_first.page_index
                && current_first.page_index == current_last.page_index
                && current_last.page_index == next_line.page_index
                && previous_line.line_index.checked_add(1) == Some(current_first.line_index)
                && current_last.line_index.checked_add(1) == Some(next_line.line_index)
        })
        .collect::<Vec<_>>();
    for index in sandwiched {
        blocks[index].role = LiquidBlockRole::Table;
        blocks[index].label = Some("Table/Figure".to_owned());
        repaired += 1;
    }
    repaired
}

pub(super) fn lm2_short_figure_table_label(text: &str) -> bool {
    let text = clean_lm2_line_text(text);
    (1..=6).contains(&word_count(&text))
        && text.len() <= 60
        && text
            .chars()
            .last()
            .is_none_or(|ch| !matches!(ch, '.' | ';' | ':'))
        && title_case_ratio(&text) >= 0.55
}

pub(super) fn lm2_numbered_table_figure_page_anchor(text: &str) -> bool {
    let normalized = normalize_text(text);
    let words = normalized.split_whitespace().collect::<Vec<_>>();
    words.windows(2).any(|pair| {
        if !matches!(
            pair[0].trim_matches(|ch: char| !ch.is_alphabetic()),
            "table" | "figure" | "fig"
        ) {
            return false;
        }
        let marker = pair[1].trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        !marker.is_empty()
            && marker.len() <= 8
            && (marker.chars().all(|ch| ch.is_ascii_digit())
                || marker.chars().all(|ch| {
                    matches!(
                        ch.to_ascii_lowercase(),
                        'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
                    )
                }))
    })
}

/// Last-resort block-level counterpart to the line guard above.  A later
/// grouping decision can preserve a recovered `N. Id.` line as Noise even
/// though its source geometry is an unambiguous note head.  Restore both the
/// block role and its source marker so the linker can use the definition.
pub(super) fn apply_hidden_numbered_note_block_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let decoded_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let owner_by_coordinate = sources
        .iter()
        .enumerate()
        .flat_map(|(source_position, source)| {
            source
                .lines
                .iter()
                .enumerate()
                .map(move |(line_position, line)| {
                    (
                        (line.page_index, line.line_index),
                        (source_position, line_position),
                    )
                })
        })
        .collect::<HashMap<_, _>>();
    // A missed footnote divider can leave a complete, tiny numbered note head
    // as a Paragraph immediately above the correctly recovered note band.  Do
    // not infer from typography alone: require the markerless physical
    // continuation on the next source row and its sequential N+1 head in the
    // same Marginalia block.  This is the boundary shape in which the first
    // rows of note N were grouped as body while the rest of N and note N+1
    // remained in the footnote band.
    let boundary_paragraph_notes = sources
        .iter()
        .filter_map(|source| {
            let block = blocks.get(source.block_index)?;
            if block.role != LiquidBlockRole::Paragraph
                || source.lines.is_empty()
                || source
                    .lines
                    .iter()
                    .any(|line| !line.note_markers.is_empty())
            {
                return None;
            }
            let marker = leading_sentineled_marker(&block.text)
                .or_else(|| leading_numeric_token_marker(&block.text))?;
            let deep_lines = source
                .lines
                .iter()
                .map(|line| {
                    line.id
                        .as_deref()
                        .and_then(|id| decoded_by_id.get(id).copied())
                })
                .collect::<Option<Vec<_>>>()?;
            let first = deep_lines.first()?;
            if deep_lines.iter().any(|line| {
                line.page_index != first.page_index
                    || line.segment_block_id != first.segment_block_id
                    || line.in_ruled_cell
                    || line.ruled_row_membership_exact
                    || line.page_table_column_like
                    || line.page_object_overlaps_image_bbox
                    || line.segment_block_table_like
                    || [
                        line.font_ratio_page_ref,
                        line.font_ratio_page,
                        line.font_ratio_doc,
                    ]
                    .into_iter()
                    .filter(|ratio| *ratio <= 0.88)
                    .count()
                        < 2
            }) {
                return None;
            }
            let last_ref = source.lines.last()?;
            let next_coordinate = (last_ref.page_index, last_ref.line_index.checked_add(1)?);
            let (next_source_position, next_line_position) =
                owner_by_coordinate.get(&next_coordinate).copied()?;
            let next_source = sources.get(next_source_position)?;
            let next_block = blocks.get(next_source.block_index)?;
            if !matches!(
                next_block.role,
                LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
            ) {
                return None;
            }
            let next_ref = next_source.lines.get(next_line_position)?;
            let next_deep = next_ref
                .id
                .as_deref()
                .and_then(|id| decoded_by_id.get(id).copied())?;
            if !next_ref.note_markers.is_empty()
                || !(next_deep.in_footnote_zone
                    || next_deep.below_footnote_divider
                    || next_deep.doc_footnote_state)
                || !next_deep.segment_block_footnote_like
                || next_deep.in_ruled_cell
                || next_deep.ruled_row_membership_exact
                || next_deep.page_table_column_like
                || next_deep.page_object_overlaps_image_bbox
            {
                return None;
            }
            let next_marker = marker.checked_add(1)?;
            next_source.lines[next_line_position + 1..]
                .iter()
                .any(|line| line.note_markers.contains(&next_marker))
                .then_some((source.block_index, marker))
        })
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    for source in sources.iter_mut() {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if let Some(marker) = boundary_paragraph_notes.get(&source.block_index).copied() {
            block.role = LiquidBlockRole::Marginalia;
            block.label = None;
            for line in &mut source.lines {
                line.role = LiquidBlockRole::Marginalia;
                line.note_markers.clear();
            }
            if let Some(first) = source.lines.first_mut() {
                first.note_markers.push(marker);
            }
            repaired += 1;
            continue;
        }
        if !matches!(
            block.role,
            LiquidBlockRole::Noise | LiquidBlockRole::Table | LiquidBlockRole::Marginalia
        ) || source.lines.len() != 1
        {
            continue;
        }
        let Some(line) = source.lines[0]
            .id
            .as_deref()
            .and_then(|id| decoded_by_id.get(id).copied())
        else {
            continue;
        };
        let provenance_marker = (source.lines[0].role == LiquidBlockRole::Marginalia)
            .then(|| {
                let block_marker = leading_numeric_token_marker(&clean_lm2_line_text(&block.text));
                source.lines[0]
                    .note_markers
                    .iter()
                    .copied()
                    .find(|marker| Some(*marker) == block_marker)
            })
            .flatten();
        let Some(marker) = provenance_marker.or_else(|| lm2_hidden_numbered_note_line_marker(line))
        else {
            continue;
        };
        block.role = LiquidBlockRole::Marginalia;
        block.label = None;
        source.lines[0].role = LiquidBlockRole::Marginalia;
        if !source.lines[0].note_markers.contains(&marker) {
            source.lines[0].note_markers.push(marker);
            source.lines[0].note_markers.sort_unstable();
        }
        repaired += 1;
    }
    repaired
}

pub(super) fn lm2_hidden_numbered_note_line_marker(line: &DeepLiquidSourceLine) -> Option<u16> {
    let physical_note = if line.page_has_footnote_divider {
        line.below_footnote_divider
    } else {
        line.in_footnote_zone || line.doc_footnote_state
    };
    let small_font = line.font_ratio_page_ref <= 0.94
        || line.font_ratio_page <= 0.94
        || line.font_ratio_doc <= 0.94;
    if !physical_note
        || !small_font
        || line.in_ruled_cell
        || line.ruled_row_membership_exact
        || (line.page_table_column_like && !line.in_footnote_zone)
        || (line.doc_repeated_edge_text
            && !line.in_footnote_zone
            && !line.page_has_footnote_divider)
    {
        return None;
    }
    leading_explicit_numbered_note_marker(&line.text)
}

/// A grouping model can merge several terse `N. Id.` definitions into one
/// Noise block. Split only provenance-backed, explicitly punctuated small-font
/// note lines; reporter volumes and table rows remain Noise.
pub(super) fn apply_hidden_numbered_note_block_split_recovery(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let decoded_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let source_by_block = sources
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<HashMap<_, _>>();
    let mut rebuilt_blocks = Vec::with_capacity(blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut repaired = 0usize;

    for (old_index, block) in blocks.iter().enumerate() {
        let Some(source) = source_by_block.get(&old_index).copied() else {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block.clone());
            rebuilt_sources.push(LiquidBlockSourceLines {
                block_index,
                lines: Vec::new(),
            });
            continue;
        };
        let candidates = if matches!(
            block.role,
            LiquidBlockRole::Noise | LiquidBlockRole::Table | LiquidBlockRole::Marginalia
        ) && source.lines.len() > 1
        {
            source
                .lines
                .iter()
                .map(|line_ref| {
                    line_ref
                        .id
                        .as_deref()
                        .and_then(|id| decoded_by_id.get(id).copied())
                        .and_then(lm2_hidden_numbered_note_line_marker)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if candidates.iter().all(Option::is_none) {
            let block_index = rebuilt_blocks.len();
            rebuilt_blocks.push(block.clone());
            let mut cloned = (*source).clone();
            cloned.block_index = block_index;
            rebuilt_sources.push(cloned);
            continue;
        }

        let mut segment_role = LiquidBlockRole::Noise;
        let mut segment_refs = Vec::<LiquidSourceLineRef>::new();
        let flush_segment =
            |role: LiquidBlockRole,
             refs: &mut Vec<LiquidSourceLineRef>,
             rebuilt_blocks: &mut Vec<LiquidBlock>,
             rebuilt_sources: &mut Vec<LiquidBlockSourceLines>| {
                if refs.is_empty() {
                    return;
                }
                let mut text = String::new();
                for line in refs.iter() {
                    append_line(&mut text, &clean_lm2_line_text(&line.text));
                }
                let block_index = rebuilt_blocks.len();
                rebuilt_blocks.push(LiquidBlock {
                    role,
                    text: collapse_whitespace(&text).trim().to_owned(),
                    label: None,
                });
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index,
                    lines: std::mem::take(refs),
                });
            };

        for (mut line_ref, marker) in source.lines.iter().cloned().zip(candidates) {
            if let Some(marker) = marker {
                flush_segment(
                    segment_role,
                    &mut segment_refs,
                    &mut rebuilt_blocks,
                    &mut rebuilt_sources,
                );
                segment_role = LiquidBlockRole::Marginalia;
                line_ref.role = LiquidBlockRole::Marginalia;
                if !line_ref.note_markers.contains(&marker) {
                    line_ref.note_markers.push(marker);
                    line_ref.note_markers.sort_unstable();
                }
                repaired += 1;
            } else if segment_role == LiquidBlockRole::Marginalia {
                line_ref.role = LiquidBlockRole::Marginalia;
            }
            segment_refs.push(line_ref);
        }
        flush_segment(
            segment_role,
            &mut segment_refs,
            &mut rebuilt_blocks,
            &mut rebuilt_sources,
        );
    }

    if repaired > 0 {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    }
    repaired
}

/// A page-level footnote-state estimate can extend upward through the last body
/// paragraph on a dense page.  Recover body-sized marginalia that appears
/// before the page's first provenance-backed note head.  The typography and
/// object-shape gates preserve true carryover notes, author notes, contents,
/// and figure/table material.
pub(super) fn apply_above_note_body_marginalia_rescue(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let decoded_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let protected_note_continuations =
        physical_open_note_continuation_blocks(blocks, sources, decoded);
    let mut first_note_line_by_page = BTreeMap::<usize, usize>::new();
    for source in sources {
        for line in &source.lines {
            if line.note_markers.is_empty()
                || leading_numeric_token_marker(&line.text)
                    .is_none_or(|marker| !line.note_markers.contains(&marker))
            {
                continue;
            }
            first_note_line_by_page
                .entry(line.page_index)
                .and_modify(|first| *first = (*first).min(line.line_index))
                .or_insert(line.line_index);
        }
    }

    let mut repaired = 0usize;
    for source in sources {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if block.role != LiquidBlockRole::Marginalia
            || source.lines.is_empty()
            || source
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty())
        {
            continue;
        }
        let Some(page_index) = source.lines.first().map(|line| line.page_index) else {
            continue;
        };
        if source
            .lines
            .iter()
            .any(|line| line.page_index != page_index)
        {
            continue;
        }
        let Some(first_note_line) = first_note_line_by_page.get(&page_index).copied() else {
            continue;
        };
        if source
            .lines
            .iter()
            .map(|line| line.line_index)
            .max()
            .is_none_or(|last| last >= first_note_line)
        {
            continue;
        }
        let deep_lines = source
            .lines
            .iter()
            .filter_map(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| decoded_by_id.get(id).copied())
            })
            .collect::<Vec<_>>();
        // Body-size rescue is vetoed only when sequential note ownership is
        // proven across the page boundary. Geometry alone is too coarse: a
        // bottom-page body block can also be labeled as a footnote segment.
        if protected_note_continuations.contains(&source.block_index) {
            continue;
        }
        if deep_lines.is_empty()
            || deep_lines.iter().any(|line| {
                line.segment_block_toc_like
                    || line.segment_block_table_like
                    || line.page_table_column_like
                    || line.in_ruled_cell
                    || line.page_object_overlaps_image_bbox
            })
        {
            continue;
        }
        let body_sized = deep_lines
            .iter()
            .filter(|line| {
                line.font_ratio_page_ref >= 0.94
                    || line.font_ratio_page >= 0.94
                    || line.font_ratio_doc >= 0.94
            })
            .count();
        if body_sized * 4 < deep_lines.len() * 3 {
            continue;
        }
        let normalized = normalize_text(&strip_callout_sentinels_lm2(&block.text));
        if word_count(&normalized) < 6
            || normalized.starts_with("author.")
            || normalized.starts_with("copyright ")
            || normalized.starts_with("doi:")
            || block
                .text
                .trim_start()
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'))
            || lm2_toc_dotleader_line(&block.text)
        {
            continue;
        }
        block.role = LiquidBlockRole::Paragraph;
        block.label = None;
        repaired += 1;
    }
    repaired
}

/// Table cells and short indented labels can be misclassified as Marginalia.
/// If such a block is physically in the body and already carries a nonleading
/// callout, keep it in body flow so the reference and cell text are not
/// stranded in the Notes tail. A same-page definition remains sufficient for
/// leading callouts, whose role is more ambiguous.
pub(super) fn apply_body_callout_marginalia_rescue(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();

    let mut repaired = 0usize;
    for source in sources {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if !matches!(
            block.role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Noise
        ) || source.lines.is_empty()
            || source.lines.iter().any(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id))
                    .is_none_or(|decoded_line| lm2_low_footnote_zone_evidence(decoded_line))
            })
        {
            continue;
        }
        let markers = sentineled_note_markers(&block.text);
        if markers.is_empty() {
            continue;
        }
        // Accepted source heads are authoritative.  A note region can occupy
        // more than half a dense page, so a genuine definition is not body
        // merely because the midpoint-based low-zone guard is false.
        if source
            .lines
            .iter()
            .any(|line| !line.note_markers.is_empty())
        {
            continue;
        }
        let nonleading_body_callout = leading_callout_marker_end(block.text.trim_start()).is_none();
        let leading_body_callout = leading_sentineled_marker(&block.text).is_some_and(|marker| {
            let first_line = source.lines.first().and_then(|line| {
                line.id
                    .as_deref()
                    .and_then(|id| line_by_id.get(id).copied())
            });
            first_line.is_some_and(|line| !lm2_note_head_zone_evidence(line))
                && lm2_strip_leading_ascii_or_decoded_callout(&block.text, marker)
                    .and_then(|remainder| remainder.chars().find(|ch| ch.is_alphanumeric()))
                    .is_some_and(char::is_alphabetic)
        });
        if nonleading_body_callout || leading_body_callout {
            block.role = LiquidBlockRole::Paragraph;
            block.label = None;
            repaired += 1;
        }
    }
    repaired
}

pub(super) fn apply_numeric_footer_furniture_suppression(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let block_by_line_id = sources
        .iter()
        .flat_map(|source| {
            source.lines.iter().filter_map(move |line| {
                line.id.as_ref().map(|id| (id.as_str(), source.block_index))
            })
        })
        .collect::<HashMap<_, _>>();
    let mut repaired = 0usize;
    for (line, _) in decoded {
        let Some(block_index) = block_by_line_id.get(line.id.as_str()).copied() else {
            continue;
        };
        let Some(block) = blocks.get_mut(block_index) else {
            continue;
        };
        if lm2_law_review_running_header_furniture(line) {
            block.role = LiquidBlockRole::Noise;
            block.label = None;
            repaired += 1;
            continue;
        }
        let Ok(number) = line.text.trim().parse::<u16>() else {
            continue;
        };
        if block.text.trim() != line.text.trim() {
            continue;
        }
        let out_of_range_furniture = number > LM2_MAX_NOTE_MARKER
            && line.segment_block_furniture_like
            && (line.line_index == 0 || line.segment_block_table_like);
        if out_of_range_furniture {
            block.role = LiquidBlockRole::Noise;
            block.label = None;
            repaired += 1;
            continue;
        }
        if line.page_height <= 0.0
            || line.bottom / line.page_height > 0.10
            || !line.centered
            || !line.segment_block_furniture_like
        {
            continue;
        }
        let horizontal_center = (line.left + line.right) * 0.5;
        if (horizontal_center - line.page_width * 0.5).abs() / line.page_width.max(1.0) > 0.06 {
            continue;
        }
        block.role = LiquidBlockRole::Noise;
        block.label = None;
        repaired += 1;
    }
    repaired
}

pub(super) fn lm2_law_review_running_header_furniture(line: &DeepLiquidSourceLine) -> bool {
    if line.line_index > 1 {
        return false;
    }
    let text = strip_callout_sentinels_lm2(&line.text);
    let normalized = normalize_text(&text);
    let words = word_count(&normalized);
    let journal = normalized.contains("law review") || normalized.contains("law journal");
    let volume_folio = normalized.contains("[vol.")
        || normalized.contains("[vol ")
        || normalized.contains(" volume ")
        || normalized
            .split_whitespace()
            .next()
            .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_digit()));
    journal
        && volume_folio
        && words <= 16
        && (line.doc_repeated_edge_text
            || line.segment_block_furniture_like
            || line.role_hint == Some(LiquidBlockRole::Noise))
}

pub(super) fn apply_adjacent_duplicate_callout_suppression(blocks: &mut [LiquidBlock]) -> usize {
    let mut repaired = 0usize;
    for block in blocks {
        let markers = sentineled_note_markers(&block.text)
            .into_iter()
            .collect::<HashSet<_>>();
        for marker in markers {
            let sentinel = format!("{CALLOUT_START}{marker}{CALLOUT_END}");
            for separator in ["", " "] {
                let duplicate = format!("{sentinel}{separator}{sentinel}");
                while let Some(position) = block.text.find(&duplicate) {
                    let end = position + duplicate.len();
                    block.text.replace_range(position..end, &sentinel);
                    repaired += 1;
                }
            }
        }
    }
    repaired
}

/// Some law-review text layers place a paragraph-ending superscript at the
/// beginning of the following extracted line.  Once block splitting runs that
/// becomes a visibly nonsensical paragraph-leading callout. Move only the
/// already-decoded callout back to the preceding paragraph; keep the prose and
/// its source provenance in place.
pub(super) fn apply_leading_callout_backfill(
    blocks: &mut [LiquidBlock],
    sources: &[LiquidBlockSourceLines],
) -> usize {
    let block_pages = sources
        .iter()
        .filter_map(|source| {
            source
                .lines
                .iter()
                .map(|line| line.page_index)
                .min()
                .map(|page| (source.block_index, page))
        })
        .collect::<BTreeMap<_, _>>();
    let mut repaired = 0usize;

    for block_index in 1..blocks.len() {
        if block_pages.get(&(block_index - 1)) != block_pages.get(&block_index)
            || block_pages.get(&block_index).is_none()
        {
            continue;
        }
        let (before, after) = blocks.split_at_mut(block_index);
        let previous = &mut before[block_index - 1];
        let current = &mut after[0];
        if previous.role != LiquidBlockRole::Paragraph
            || current.role != LiquidBlockRole::Paragraph
            || !lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text))
        {
            continue;
        }
        let Some(marker_end) = leading_callout_marker_end(&current.text) else {
            continue;
        };
        let marker = current.text[..marker_end].to_owned();
        let remaining = current.text[marker_end..].trim_start();
        if remaining.is_empty()
            || !remaining
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || matches!(ch, '"' | '\'' | '('))
        {
            continue;
        }
        previous.text = format!("{}{}", previous.text.trim_end(), marker);
        current.text = remaining.to_owned();
        repaired += 1;
    }
    repaired
}

pub(super) fn leading_callout_marker_end(text: &str) -> Option<usize> {
    let tail = text.strip_prefix(CALLOUT_START)?;
    let end_in_tail = tail.find(CALLOUT_END)?;
    let digits = &tail[..end_in_tail];
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(CALLOUT_START.len_utf8() + end_in_tail + CALLOUT_END.len_utf8())
}

pub(super) fn leading_sentineled_note_head_marker(line: &DeepLiquidSourceLine) -> Option<u16> {
    let marker = leading_sentineled_marker(&line.text)?;
    (lm2_note_head_zone_evidence(line)
        || lm2_explicit_small_font_sentineled_note_head(line, marker))
    .then_some(marker)
}

/// Journals with unusually deep note regions can defeat both the divider and
/// learned-zone detectors.  An encoded leading number immediately followed by
/// printed note punctuation in unmistakably smaller type is still strong head
/// evidence.  Body callouts use a space after the sentinel, not `.`/`)`/`]`.
pub(super) fn lm2_explicit_small_font_sentineled_note_head(
    line: &DeepLiquidSourceLine,
    marker: u16,
) -> bool {
    let trimmed = line.text.trim_start();
    let Some(end) = leading_callout_marker_end(trimmed) else {
        return false;
    };
    let encoded = &trimmed[CALLOUT_START.len_utf8()..end - CALLOUT_END.len_utf8()];
    if encoded.parse::<u16>().ok() != Some(marker) {
        return false;
    }
    let mut tail = trimmed[end..].chars();
    if !matches!(tail.next(), Some('.') | Some(')') | Some(']'))
        || tail.next().is_some_and(|ch| !ch.is_whitespace())
    {
        return false;
    }
    let small_font = line.font_ratio_page_ref <= 0.90
        || line.font_ratio_doc <= 0.90
        || line.font_ratio_page <= 0.86;
    small_font
        && line.role_hint != Some(LiquidBlockRole::Contents)
        && !line.in_ruled_cell
        && !line.page_table_column_like
        && !line.doc_repeated_edge_text
}

/// Evidence that a leading marker is physically a definition head.  This is
/// intentionally broader than `lm2_low_footnote_zone_evidence`: a dense page
/// can devote more than half its height to notes, so a real head may sit above
/// the page midpoint even when the divider detector misses the rule.  Small
/// typography plus the extracted footnote-block shape is enough in that case;
/// body superscripts do not have that combination.
pub(super) fn lm2_note_head_zone_evidence(line: &DeepLiquidSourceLine) -> bool {
    if line.page_has_footnote_divider {
        return line.below_footnote_divider;
    }
    if !(line.in_footnote_zone || line.doc_footnote_state) {
        return false;
    }
    let physically_low = line.page_height > 0.0 && line.top <= line.page_height * 0.50;
    let small_note_font = line.font_ratio_page_ref <= 0.94
        || line.font_ratio_page <= 0.94
        || line.font_ratio_doc <= 0.94;
    physically_low
        || (small_note_font
            && line.segment_block_footnote_like
            && !line.in_ruled_cell
            && !line.page_table_column_like)
}

pub(super) fn leading_sentineled_marker(text: &str) -> Option<u16> {
    let trimmed = text.trim_start();
    let end = leading_callout_marker_end(trimmed)?;
    let marker = trimmed[CALLOUT_START.len_utf8()..end - CALLOUT_END.len_utf8()]
        .parse::<u16>()
        .ok()?;
    (1..=LM2_MAX_NOTE_MARKER)
        .contains(&marker)
        .then_some(marker)
}

pub(super) fn apply_deferred_marginalia_reflow_once(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    if blocks.len() < 3 {
        return 0;
    }
    let mut source_lines_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut reflowed = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        if index + 2 < old_blocks.len()
            && lm2_should_bridge_leading_callout(
                &old_blocks[index],
                &old_blocks[index + 1],
                &old_blocks[index + 2],
                &source_lines_by_block,
                index,
            )
        {
            let mut merged = old_blocks[index].clone();
            append_line(&mut merged.text, &old_blocks[index + 1].text);
            let marker_end = leading_callout_marker_end(&old_blocks[index + 2].text)
                .expect("bridge predicate requires a leading callout");
            merged
                .text
                .push_str(&old_blocks[index + 2].text[..marker_end]);
            let mut continuation = old_blocks[index + 2].clone();
            continuation.text = continuation.text[marker_end..].trim_start().to_owned();

            let merged_index = rebuilt_blocks.len();
            rebuilt_blocks.push(merged);
            let mut merged_refs = source_lines_by_block.remove(&index).unwrap_or_default();
            merged_refs.extend(
                source_lines_by_block
                    .remove(&(index + 1))
                    .unwrap_or_default(),
            );
            if !merged_refs.is_empty() {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index: merged_index,
                    lines: merged_refs,
                });
            }

            let continuation_index = rebuilt_blocks.len();
            rebuilt_blocks.push(continuation);
            if let Some(lines) = source_lines_by_block.remove(&(index + 2))
                && !lines.is_empty()
            {
                rebuilt_sources.push(LiquidBlockSourceLines {
                    block_index: continuation_index,
                    lines,
                });
            }
            reflowed += 1;
            index += 3;
            continue;
        }

        if old_blocks[index].role == LiquidBlockRole::Paragraph {
            let mut note_end = index + 1;
            while note_end < old_blocks.len()
                && lm2_reflow_deferred_note_role(old_blocks[note_end].role)
            {
                note_end += 1;
            }
            if note_end > index + 1
                && note_end < old_blocks.len()
                && old_blocks[note_end].role == LiquidBlockRole::Paragraph
                && lm2_should_reflow_deferred_marginalia(
                    &old_blocks[index].text,
                    &old_blocks[note_end].text,
                )
            {
                let mut merged_paragraph = old_blocks[index].clone();
                append_line(&mut merged_paragraph.text, &old_blocks[note_end].text);
                let block_index = rebuilt_blocks.len();
                rebuilt_blocks.push(merged_paragraph);

                let mut merged_refs = source_lines_by_block.remove(&index).unwrap_or_default();
                merged_refs.extend(source_lines_by_block.remove(&note_end).unwrap_or_default());
                if !merged_refs.is_empty() {
                    rebuilt_sources.push(LiquidBlockSourceLines {
                        block_index,
                        lines: merged_refs,
                    });
                }

                for note_index in (index + 1)..note_end {
                    let block_index = rebuilt_blocks.len();
                    rebuilt_blocks.push(old_blocks[note_index].clone());
                    if let Some(lines) = source_lines_by_block.remove(&note_index)
                        && !lines.is_empty()
                    {
                        rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
                    }
                }
                reflowed += 1;
                index = note_end + 1;
                continue;
            }
        }

        let block_index = rebuilt_blocks.len();
        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_lines_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }

    if reflowed > 0 {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    } else {
        *blocks = old_blocks;
    }
    reflowed
}

pub(super) fn lm2_should_bridge_leading_callout(
    before: &LiquidBlock,
    bridge: &LiquidBlock,
    after: &LiquidBlock,
    sources: &BTreeMap<usize, Vec<LiquidSourceLineRef>>,
    before_index: usize,
) -> bool {
    if before.role != LiquidBlockRole::Paragraph
        || after.role != LiquidBlockRole::Paragraph
        || !lm2_reflow_paragraph_is_visibly_open(&before.text)
        || leading_callout_marker_end(&after.text).is_none()
        || leading_numeric_token_marker(&bridge.text).is_some()
    {
        return false;
    }
    let page = |index: usize| {
        sources
            .get(&index)
            .and_then(|lines| lines.iter().map(|line| line.page_index).min())
    };
    if page(before_index).is_none()
        || page(before_index) != page(before_index + 1)
        || page(before_index) != page(before_index + 2)
    {
        return false;
    }

    let after_without_marker = &after.text[leading_callout_marker_end(&after.text).unwrap()..];
    let after_first = after_without_marker.trim_start().chars().next();
    match bridge.role {
        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote => {
            lm2_reflow_starts_like_continuation(&bridge.text)
                && after_first.is_some_and(char::is_alphabetic)
        }
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading => {
            before
                .text
                .split_whitespace()
                .last()
                .is_some_and(|word| matches!(word, "In" | "in" | "See" | "see"))
                && bridge.text.trim_end().ends_with(',')
                && after_first.is_some_and(char::is_lowercase)
        }
        _ => false,
    }
}

pub(super) fn lm2_reflow_deferred_note_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
    )
}

pub(super) fn lm2_should_reflow_deferred_marginalia(before: &str, after: &str) -> bool {
    let before = before.trim();
    let after = after.trim();
    if before.is_empty() || after.is_empty() {
        return false;
    }
    if !lm2_reflow_paragraph_is_visibly_open(before) {
        return false;
    }
    lm2_reflow_starts_like_continuation(after)
}

pub(super) fn lm2_reflow_paragraph_is_visibly_open(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with('-') || !lm2_blocksplit_ends_like_paragraph(trimmed)
}

pub(super) fn lm2_reflow_starts_like_continuation(text: &str) -> bool {
    let trimmed = text.trim_start();
    let first = trimmed
        .chars()
        .find(|ch| !matches!(ch, '"' | '\'' | '“' | '‘' | '(' | '['));
    first.is_some_and(|ch| {
        ch.is_ascii_lowercase()
            || matches!(
                ch,
                ',' | ';' | ':' | ')' | ']' | '”' | '’' | '-' | '–' | '—'
            )
    })
}

pub(super) fn lm2_blocksplit_should_split(
    previous_ref: Option<&LiquidSourceLineRef>,
    line_ref: &LiquidSourceLineRef,
    previous_role: Option<LiquidBlockRole>,
    role: LiquidBlockRole,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let Some(previous_ref) = previous_ref else {
        return false;
    };
    let Some(previous_role) = previous_role else {
        return false;
    };
    if role != previous_role {
        return true;
    }
    if !matches!(
        role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Abstract | LiquidBlockRole::Marginalia
    ) {
        return false;
    }
    if lm2_blocksplit_divider_like(&line_ref.text)
        || lm2_blocksplit_divider_like(&previous_ref.text)
    {
        return true;
    }

    let left = line_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id))
        .map(|line| line.left)
        .unwrap_or_default();
    let previous_left = previous_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id))
        .map(|line| line.left)
        .unwrap_or_default();
    let indent_increase = left - previous_left;

    if matches!(role, LiquidBlockRole::Paragraph | LiquidBlockRole::Abstract)
        && indent_increase >= 8.0
        && lm2_blocksplit_ends_like_paragraph(&previous_ref.text)
    {
        return true;
    }
    if role == LiquidBlockRole::Paragraph
        && lm2_blocksplit_tall_row_paragraph_start(previous_ref, line_ref, line_by_id)
    {
        return true;
    }
    role == LiquidBlockRole::Marginalia
        && indent_increase >= 8.0
        && lm2_blocksplit_numbered_marginalia_start(&line_ref.text)
}

/// Some born-digital law-review PDFs expose two visual lines as one text row.
/// When that happens, the second (flush-left) visual line hides the first-line
/// indent in the row's bounding box. Recover the paragraph break only when the
/// previous visual line is demonstrably short and sentence-final and the new
/// row is tall enough to contain multiple visual lines.
pub(super) fn lm2_blocksplit_tall_row_paragraph_start(
    previous_ref: &LiquidSourceLineRef,
    line_ref: &LiquidSourceLineRef,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let (Some(previous), Some(current)) = (
        previous_ref.id.as_deref().and_then(|id| line_by_id.get(id)),
        line_ref.id.as_deref().and_then(|id| line_by_id.get(id)),
    ) else {
        return false;
    };
    lm2_tall_row_paragraph_start(previous, current)
}

pub(super) fn lm2_tall_row_paragraph_start(
    previous: &DeepLiquidSourceLine,
    current: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != current.page_index
        || current.synthetic_text_geometry
        || current.font_height <= 0.0
        || (current.top - current.bottom).abs() < current.font_height * 1.8
        || !lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text))
        || !lm2_blocksplit_starts_like_sentence(&current.text)
    {
        return false;
    }

    let previous_width = (previous.right - previous.left).abs();
    let visibly_short = previous.page_width > 0.0 && previous_width < previous.page_width * 0.55;
    let soft_wrap_tail_words = previous
        .text
        .rsplit_once('\u{0002}')
        .map(|(_, tail)| tail.split_whitespace().count())
        .unwrap_or(usize::MAX);
    visibly_short || soft_wrap_tail_words <= 6
}

pub(super) fn lm2_blocksplit_starts_like_sentence(text: &str) -> bool {
    text.trim_start()
        .chars()
        .find(|ch| !matches!(ch, '"' | '\'' | '\u{2018}' | '\u{201c}' | '(' | '['))
        .is_some_and(char::is_uppercase)
}

pub(super) fn lm2_blocksplit_ends_like_paragraph(text: &str) -> bool {
    let trimmed = text.trim_end_matches('\u{0002}').trim_end();
    let mut chars = trimmed.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if matches!(last, '"' | '\'' | ')' | ']') {
        chars
            .next()
            .is_some_and(|ch| matches!(ch, '.' | '!' | '?') || ch.is_ascii_digit())
    } else {
        matches!(last, '.' | '!' | '?') || last.is_ascii_digit()
    }
}

pub(super) fn lm2_blocksplit_divider_like(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.chars().count() >= 12
        && trimmed
            .chars()
            .all(|ch| matches!(ch, '-' | '–' | '—' | '_') || ch.is_whitespace())
}

pub(super) fn lm2_blocksplit_numbered_marginalia_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut digits = 0usize;
    let mut chars = trimmed.chars().peekable();
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) && digits < 4 {
        chars.next();
        digits += 1;
    }
    if digits == 0 || chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }
    if !matches!(chars.next(), Some('.') | Some(')')) {
        return false;
    }
    chars.next().is_some_and(|ch| ch.is_whitespace())
}

pub(super) fn grouping_line_group_index(
    grouping: Option<&Lm2PymupdfGroupingResponse>,
) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    let Some(grouping) = grouping else {
        return out;
    };
    for (block_index, block) in grouping.blocks.iter().enumerate() {
        let group_index = block.block_index.unwrap_or(block_index);
        for line_id in &block.source_line_ids {
            out.insert(line_id.clone(), group_index);
        }
    }
    out
}

#[derive(Debug)]
pub(super) struct Lm2RecoveredTitle {
    pub(super) title: String,
    pub(super) lines: Vec<DeepLiquidSourceLine>,
    pub(super) authoritative_display: bool,
}

pub(super) fn flush_block(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    text: &mut String,
    refs: &mut Vec<LiquidSourceLineRef>,
    role: LiquidBlockRole,
) {
    let cleaned = collapse_whitespace(text).trim().to_owned();
    if cleaned.is_empty() {
        text.clear();
        refs.clear();
        return;
    }
    let block_index = blocks.len();
    blocks.push(LiquidBlock {
        role,
        text: cleaned,
        label: (role == LiquidBlockRole::Table).then(|| "Table/Figure".to_owned()),
    });
    if !refs.is_empty() {
        sources.push(LiquidBlockSourceLines {
            block_index,
            lines: std::mem::take(refs),
        });
    }
    text.clear();
}

pub(super) fn append_line(text: &mut String, line: &str) {
    let line = clean_lm2_line_text(line);
    if line.is_empty() {
        return;
    }
    if !text.is_empty() {
        if should_join_dehyphenated(text, &line) {
            while text.ends_with(char::is_whitespace) || text.ends_with('-') {
                text.pop();
            }
        } else if should_join_preserved_hyphen(text, &line) {
            while text.ends_with(char::is_whitespace) {
                text.pop();
            }
        } else if text.trim_end().ends_with(['\u{2013}', '\u{2014}']) {
            // Em/en dashes are closed up in the source typography. If PDF line
            // extraction breaks immediately after one, a generic inter-line
            // space would manufacture `word— next`.
            while text.ends_with(char::is_whitespace) {
                text.pop();
            }
        } else {
            text.push(' ');
        }
    }
    text.push_str(&line);
}

pub(super) fn append_standalone_marker_to_line(text: &mut String, marker: &str) {
    let marker = clean_lm2_line_text(marker);
    if marker.is_empty() {
        return;
    }
    if let Ok(number) = marker.parse::<u16>()
        && (1..=LM2_MAX_NOTE_MARKER).contains(&number)
    {
        while text.ends_with(char::is_whitespace) {
            text.pop();
        }
        text.push(CALLOUT_START);
        text.push_str(&number.to_string());
        text.push(CALLOUT_END);
        return;
    }
    if !text.is_empty() {
        while text.ends_with(char::is_whitespace) {
            text.pop();
        }
        text.push(' ');
    }
    text.push_str(&marker);
}

pub(super) fn lm2_should_attach_standalone_marker_to_current_block(
    current_role: LiquidBlockRole,
    role: LiquidBlockRole,
    previous: Option<&DeepLiquidSourceLine>,
    marker: &DeepLiquidSourceLine,
) -> bool {
    ((current_role == LiquidBlockRole::Paragraph && role == LiquidBlockRole::Paragraph)
        || (current_role == LiquidBlockRole::Marginalia && role == LiquidBlockRole::Marginalia))
        && lm2_looks_like_standalone_marker_fragment(&marker.text)
        && previous.is_some_and(|previous| lm2_marker_can_attach_to_previous_line(marker, previous))
}

pub(super) fn lm2_looks_like_standalone_marker_fragment(text: &str) -> bool {
    let text = text.trim();
    (1..=4).contains(&text.len()) && text.chars().all(|ch| ch.is_ascii_digit())
}

pub(super) fn lm2_marker_can_attach_to_previous_line(
    marker: &DeepLiquidSourceLine,
    previous: &DeepLiquidSourceLine,
) -> bool {
    if marker.page_index != previous.page_index {
        return false;
    }
    if marker.font_height > previous.font_height * 0.80 {
        return false;
    }
    let marker_center = (marker.top + marker.bottom) * 0.5;
    let previous_center = (previous.top + previous.bottom) * 0.5;
    let vertical_delta = (marker_center - previous_center).abs();
    let vertical_overlap = marker.bottom <= previous.top && marker.top >= previous.bottom;
    let plausible_superscript_offset =
        vertical_overlap && vertical_delta <= previous.font_height * 0.75;
    let page_width = previous.page_width.max(marker.page_width).max(1.0);
    let horizontal_tolerance = (page_width * 0.007).max(2.0);
    let horizontal_close =
        marker.left >= previous.right - 2.0 && marker.left <= previous.right + horizontal_tolerance;
    let previous_can_host_marker = previous.text.trim_end().chars().last().is_some_and(|ch| {
        ch.is_ascii_alphabetic()
            || matches!(
                ch,
                '.' | '?'
                    | '!'
                    | ','
                    | ';'
                    | ':'
                    | '"'
                    | '\''
                    | '\u{2019}'
                    | '\u{201d}'
                    | ')'
                    | ']'
            )
    });
    plausible_superscript_offset && horizontal_close && previous_can_host_marker
}
