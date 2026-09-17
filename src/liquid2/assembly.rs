//! Block building from decoded lines, grouping, and the note-start longest-ascending-run selector.

use super::*;

#[cfg(any(feature = "devtools", test))]
pub(super) fn build_lm2_blocks(
    fallback_title: &str,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> (String, Vec<LiquidBlock>, Vec<LiquidBlockSourceLines>) {
    build_lm2_blocks_with_grouping(fallback_title, decoded, None, &[])
}

pub(super) fn build_lm2_blocks_with_grouping(
    fallback_title: &str,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    grouping: Option<&Lm2PymupdfGroupingResponse>,
    article_spans: &[ArticleSpan],
) -> (String, Vec<LiquidBlock>, Vec<LiquidBlockSourceLines>) {
    let mut blocks = Vec::new();
    let mut sources = Vec::new();
    let mut title = fallback_title.trim().to_owned();
    let repository_covers = repository_cover_pages(decoded);
    let repository_title = repository_citation_title(decoded, &repository_covers);
    let recovered_title = lm2_recover_leading_source_title(decoded);
    let recovered_line_ids = recovered_title
        .as_ref()
        .map(|recovered| {
            recovered
                .lines
                .iter()
                .map(|line| line.id.clone())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut current_text = String::new();
    let mut current_refs = Vec::new();
    let mut current_role = LiquidBlockRole::Paragraph;
    let mut current_last_line: Option<DeepLiquidSourceLine> = None;
    let note_start_ids = note_start_line_ids(decoded, article_spans);
    let line_group_index = grouping_line_group_index(grouping);
    let mut current_group_index: Option<usize> = None;

    if let Some(recovered) = &recovered_title {
        let block_index = blocks.len();
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Title,
            text: recovered.title.clone(),
            label: None,
        });
        sources.push(LiquidBlockSourceLines {
            block_index,
            lines: recovered
                .lines
                .iter()
                .map(|line| line_ref(line, LiquidBlockRole::Title))
                .collect(),
        });
    }

    for (line, action) in decoded {
        if recovered_line_ids.contains(&line.id) {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_last_line = None;
            current_group_index = None;
            continue;
        }
        if *action == Lm2Action::HideNoise {
            if matches!(
                line.role_hint,
                Some(LiquidBlockRole::Table | LiquidBlockRole::Caption)
            ) {
                flush_block(
                    &mut blocks,
                    &mut sources,
                    &mut current_text,
                    &mut current_refs,
                    current_role,
                );
                let text = clean_lm2_line_text(&line.text);
                if !text.is_empty() {
                    let block_index = blocks.len();
                    let role = line.role_hint.expect("table/figure role remains available");
                    blocks.push(LiquidBlock {
                        role,
                        text,
                        label: (role == LiquidBlockRole::Table).then(|| "Table/Figure".to_owned()),
                    });
                    sources.push(LiquidBlockSourceLines {
                        block_index,
                        lines: vec![line_ref(line, role)],
                    });
                }
                current_last_line = None;
                current_group_index = None;
                continue;
            }
            // Otherwise fall through as Noise rather than dropping the line.
            // Classification used to delete here, with nothing downstream able
            // to reach the text again; on a 1926 volume that removed 1,088
            // lines, a fifth of the article. Deletion is now an explicit
            // decision the generator makes about furniture, where it can be
            // seen and measured.
        }
        let role = if *action == Lm2Action::HideNoise {
            LiquidBlockRole::Noise
        } else {
            role_for_decoded_line(line, *action, blocks.is_empty())
        };
        if lm2_should_attach_standalone_marker_to_current_block(
            current_role,
            role,
            current_last_line.as_ref(),
            line,
        ) {
            append_standalone_marker_to_line(&mut current_text, &line.text);
            current_refs.push(line_ref(line, current_role));
            continue;
        }
        let force_standalone = matches!(
            role,
            LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        );
        let starts_new_note = current_role == LiquidBlockRole::Marginalia
            && role == LiquidBlockRole::Marginalia
            && !current_text.is_empty()
            && note_start_ids.contains(&line.id);
        let starts_new_noise_page = current_role == LiquidBlockRole::Noise
            && role == LiquidBlockRole::Noise
            && current_last_line
                .as_ref()
                .is_some_and(|previous| previous.page_index != line.page_index);
        let starts_new_paragraph = current_role == role
            && matches!(role, LiquidBlockRole::Paragraph | LiquidBlockRole::Abstract)
            && current_last_line.as_ref().is_some_and(|previous| {
                if let Some(group_index) = line_group_index.get(&line.id).copied() {
                    if previous.page_index == line.page_index {
                        current_group_index != Some(group_index)
                            || paragraph_boundary(previous, line)
                    } else {
                        paragraph_boundary(previous, line)
                    }
                } else {
                    paragraph_boundary(previous, line)
                }
            });
        if current_text.is_empty() {
            current_role = role;
            current_group_index = line_group_index.get(&line.id).copied();
        } else if role != current_role
            || force_standalone
            || starts_new_note
            || starts_new_noise_page
            || starts_new_paragraph
        {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_role = role;
            current_group_index = line_group_index.get(&line.id).copied();
        }
        append_line(&mut current_text, &line.text);
        current_refs.push(line_ref_with_note_start(
            line,
            role,
            note_start_ids.contains(&line.id),
        ));
        current_last_line = Some(line.clone());
        if force_standalone {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_last_line = None;
            current_group_index = None;
        }
    }
    flush_block(
        &mut blocks,
        &mut sources,
        &mut current_text,
        &mut current_refs,
        current_role,
    );

    if let Some(fallback) = lm2_fallback_title_from_blocks(&blocks) {
        let short_metadata_title = word_count(&title) <= 4
            && word_count(&fallback) >= word_count(&title).saturating_add(4)
            && uppercase_ratio(&fallback) >= 0.62
            && title_case_ratio(&title) >= 0.75;
        if title.is_empty()
            || looks_like_filename_fallback_title(&title)
            || short_metadata_title
            || lm2_recovered_title_is_better(&fallback, &title)
        {
            title = fallback;
        }
    }
    if let Some(recovered) = recovered_title
        && (title.is_empty()
            || looks_like_filename_fallback_title(&title)
            || recovered.authoritative_display
            || lm2_recovered_title_is_better(&recovered.title, &title))
    {
        title = recovered.title;
    }
    if let Some(mut repository_title) = repository_title
        && (title.is_empty()
            || looks_like_filename_fallback_title(&title)
            || lm2_recovered_title_is_better(&repository_title, &title)
            || title_word_overlap(&repository_title, &title) >= 0.80
                && uppercase_ratio(&title) >= 0.70)
    {
        if title.trim_end().ends_with('?')
            && !repository_title
                .trim_end()
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '?' | '!' | '.'))
        {
            repository_title.push('?');
        }
        title = repository_title;
    }
    if title.is_empty() {
        title = "Untitled document".to_owned();
    }
    (title, blocks, sources)
}

/// Line ids that genuinely open a footnote, decided within each article.
///
/// Whether a line opens a note or continues one is not decidable on that line.
/// A citation volume (`106 VA. L. REV. 611`) and a numbered list inside a note
/// both look like note heads locally. A law-review article numbers its notes
/// consecutively, so the true heads form the longest ascending subsequence.
///
/// The article scope is essential: a bound volume restarts at 1 for every
/// article. Running this optimization across the whole PDF suppresses the note
/// heads in all but one article.
pub(super) fn note_start_line_ids(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    article_spans: &[ArticleSpan],
) -> HashSet<String> {
    let scoped = note_start_line_ids_for_scope(decoded, article_spans);
    let global = if article_spans.is_empty() {
        HashSet::new()
    } else {
        note_start_line_ids_for_scope(decoded, &[])
    };
    let mut starts = merge_article_and_global_note_starts(scoped, global, article_spans);
    starts.extend(same_page_body_referenced_note_heads(decoded));
    starts
}

pub(super) fn rescue_omitted_keep_source_lines(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) {
    let assembled = sources
        .iter()
        .flat_map(|source| source.lines.iter().filter_map(|line| line.id.clone()))
        .collect::<HashSet<_>>();
    let keep_ids = decoded
        .iter()
        .filter(|(_, action)| *action == Lm2Action::Keep)
        .map(|(line, _)| line.id.as_str());
    for id in omitted_keep_source_ids(keep_ids, &assembled) {
        let Some((line, _)) = decoded.iter().find(|(line, _)| line.id == id) else {
            continue;
        };
        let text = clean_lm2_line_text(&line.text);
        if text.is_empty() {
            continue;
        }
        let block_index = blocks.len();
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text,
            label: None,
        });
        sources.push(LiquidBlockSourceLines {
            block_index,
            lines: vec![line_ref(line, LiquidBlockRole::Paragraph)],
        });
    }
}

/// A citation-shaped definition such as `15 28 U.S.C. ...` is ambiguous in
/// isolation, but it is a reliable note head when the same number is already
/// encoded as a body callout on that page. This recovers isolated definitions
/// without admitting unrelated reporter citations elsewhere in the notes.
pub(super) fn same_page_body_referenced_note_heads(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashSet<String> {
    let mut body_markers: BTreeMap<usize, HashSet<u16>> = BTreeMap::new();
    let mut contextual_note_heads: BTreeMap<usize, HashSet<u16>> = BTreeMap::new();
    for (line, action) in decoded {
        if *action == Lm2Action::Marginalia {
            contextual_note_heads
                .entry(line.page_index)
                .or_default()
                .extend(sentineled_note_head_markers_in_context(&line.text));
        }
        if *action != Lm2Action::Keep
            || line.in_footnote_zone
            || line.below_footnote_divider
            || line.doc_footnote_state
            || line.role_hint == Some(LiquidBlockRole::Marginalia)
        {
            continue;
        }
        body_markers
            .entry(line.page_index)
            .or_default()
            .extend(sentineled_note_markers(&line.text));
    }

    decoded
        .iter()
        .filter_map(|item @ (line, action)| {
            if *action != Lm2Action::Marginalia {
                return None;
            }
            let marker = numeric_note_head_candidate(item)?;
            if contextual_note_heads
                .get(&line.page_index)
                .is_some_and(|markers| markers.contains(&marker))
            {
                // Prefer the protected contextual head over a bare same-page
                // pincite that happens to equal a body callout.
                return None;
            }
            body_markers
                .get(&line.page_index)
                .is_some_and(|markers| markers.contains(&marker))
                .then(|| line.id.clone())
        })
        .collect()
}

pub(super) fn note_start_line_ids_for_scope(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    article_spans: &[ArticleSpan],
) -> HashSet<String> {
    // A dedicated Footnotes/Endnotes section can provide an unusually strong
    // document-level marker sequence. Once that sequence is present, it is
    // better evidence than a citation-shaped line such as `55 Fed. Reg. ...`
    // inside another note. Make the preference per article so bound volumes can
    // still restart numbering independently.
    let mut document_markers_by_article: BTreeMap<usize, Vec<(usize, usize, u16)>> =
        BTreeMap::new();
    for (line, action) in decoded {
        if *action != Lm2Action::Marginalia || line.doc_note_marker == 0 {
            continue;
        }
        let article_index =
            article_index_at(article_spans, line.page_index, line.line_index).unwrap_or(usize::MAX);
        document_markers_by_article
            .entry(article_index)
            .or_default()
            .push((line.page_index, line.line_index, line.doc_note_marker));
    }
    let authoritative_document_marker_articles = document_markers_by_article
        .into_iter()
        .filter_map(|(article_index, mut markers)| {
            // Decoder storage order is not a reading-order contract. The
            // explicit-endnote detector itself uses page/line order, so apply
            // the same ordering before judging its recovered sequence.
            markers.sort_by_key(|(page_index, line_index, _)| (*page_index, *line_index));
            let markers = markers
                .into_iter()
                .map(|(_, _, marker)| marker)
                .collect::<Vec<_>>();
            let strong_sequence = markers.len() >= DOCUMENT_NOTE_SEQUENCE_MIN_CANDIDATES
                && markers.first().is_some_and(|marker| *marker <= 5)
                && markers
                    .windows(2)
                    .all(|pair| pair[1] > pair[0] && pair[1].saturating_sub(pair[0]) <= 3);
            strong_sequence.then_some(article_index)
        })
        .collect::<HashSet<_>>();

    let mut by_article: BTreeMap<usize, Vec<(usize, usize, &String, u16)>> = BTreeMap::new();
    for (index, (line, action)) in decoded.iter().enumerate() {
        if *action != Lm2Action::Marginalia {
            continue;
        }
        let article_index =
            article_index_at(article_spans, line.page_index, line.line_index).unwrap_or(usize::MAX);
        let number = if authoritative_document_marker_articles.contains(&article_index) {
            (line.doc_note_marker > 0).then_some(line.doc_note_marker)
        } else {
            (line.doc_note_marker > 0)
                .then_some(line.doc_note_marker)
                .or_else(|| leading_sentineled_note_head_marker(line))
                .or_else(|| note_head_marker(&line.text))
                .or_else(|| sequence_numeric_note_head_marker(decoded, index))
        };
        let Some(number) = number else {
            continue;
        };
        by_article.entry(article_index).or_default().push((
            line.page_index,
            line.line_index,
            &line.id,
            number,
        ));
    }

    let mut starts = HashSet::new();
    for mut candidates in by_article.into_values() {
        // Decoder storage order is not a reading-order contract. In old bound
        // volumes, leaving this unsorted allowed a late reporter page such as
        // `875` to enter the longest ascending subsequence ahead of the real
        // local note run.
        candidates.sort_by_key(|(page_index, line_index, _, _)| (*page_index, *line_index));
        if candidates.len() < NOTE_SEQUENCE_MIN_CANDIDATES {
            starts.extend(candidates.into_iter().map(|(_, _, id, _)| id.clone()));
            continue;
        }
        let numbers = candidates
            .iter()
            .map(|(_, _, _, number)| *number)
            .collect::<Vec<_>>();
        let keep = longest_ascending_run(&numbers);
        // A run explaining less than half the candidates is not strong enough
        // evidence to reinterpret local note heads.
        if keep.len() * 2 < candidates.len() {
            starts.extend(candidates.into_iter().map(|(_, _, id, _)| id.clone()));
            continue;
        }
        starts.extend(
            candidates
                .into_iter()
                .enumerate()
                .filter(|(position, _)| keep.contains(position))
                .map(|(_, (_, _, id, _))| id.clone()),
        );
    }
    starts
}

/// Accept citation-shaped note definitions only when neighboring lower-zone
/// lines establish a consecutive note-head run. This recovers definitions
/// such as `128 8 U.S.C. ...` and `129 142 S. Ct. ...` without treating an
/// isolated citation volume as a footnote number.
pub(super) fn sequence_numeric_note_head_marker(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    index: usize,
) -> Option<u16> {
    let marker = numeric_note_head_candidate(decoded.get(index)?)?;

    for start in index.saturating_sub(2)..=index {
        let Some(window) = decoded.get(start..start.saturating_add(3)) else {
            continue;
        };
        if index < start || index >= start + window.len() {
            continue;
        }
        let Some(first) = numeric_note_head_candidate(&window[0]) else {
            continue;
        };
        let Some(second) = numeric_note_head_candidate(&window[1]) else {
            continue;
        };
        let Some(third) = numeric_note_head_candidate(&window[2]) else {
            continue;
        };
        let same_page = window
            .iter()
            .all(|(line, _)| line.page_index == window[0].0.page_index);
        let adjacent = window
            .windows(2)
            .all(|pair| pair[1].0.line_index == pair[0].0.line_index + 1);
        if same_page
            && adjacent
            && second == first.saturating_add(1)
            && third == second.saturating_add(1)
        {
            return Some(marker);
        }
    }

    for neighbor_index in [index.checked_sub(1), index.checked_add(1)]
        .into_iter()
        .flatten()
    {
        let Some((neighbor, neighbor_action)) = decoded.get(neighbor_index) else {
            continue;
        };
        if *neighbor_action != Lm2Action::Marginalia
            || neighbor.page_index != decoded[index].0.page_index
            || note_head_marker(&neighbor.text).is_none()
        {
            continue;
        }
        let Some(neighbor_marker) = leading_numeric_token_marker(&neighbor.text) else {
            continue;
        };
        if marker.abs_diff(neighbor_marker) == 1 {
            return Some(marker);
        }
    }
    None
}

pub(super) fn numeric_note_head_candidate(item: &(DeepLiquidSourceLine, Lm2Action)) -> Option<u16> {
    let (line, action) = item;
    if *action != Lm2Action::Marginalia
        || !(line.in_footnote_zone
            || line.below_footnote_divider
            || line.doc_footnote_state
            || line.role_hint == Some(LiquidBlockRole::Marginalia))
    {
        return None;
    }
    leading_numeric_token_marker(&line.text)
}

pub(super) fn leading_numeric_token_marker(text: &str) -> Option<u16> {
    leading_numeric_token_marker_with_len(text).map(|(marker, _)| marker)
}

/// `leading_numeric_token_marker` plus the byte length of the digit run that
/// was matched (after `trim_start`). Callers that strip the marker from the
/// line must use this length: `marker.to_string().len()` drops leading zeros
/// and so slices the remainder at the wrong offset for `007 ...`.
pub(super) fn leading_numeric_token_marker_with_len(text: &str) -> Option<(u16, usize)> {
    let trimmed = text.trim_start();
    let digits = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let remainder = &trimmed[digits.len()..];
    if remainder.starts_with('.')
        && remainder
            .chars()
            .nth(1)
            .is_some_and(|ch| ch.is_ascii_digit())
    {
        // `6.3 Heading` is a decimal section number, not integer note 6.
        // A true `6. 3 U.S.C. ...` note head retains the separating space.
        return None;
    }
    if remainder
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_whitespace() && !matches!(ch, '.' | ')' | ']'))
    {
        return None;
    }
    let marker = digits.parse::<u16>().ok()?;
    (1..=LM2_MAX_NOTE_MARKER)
        .contains(&marker)
        .then_some((marker, digits.len()))
}

pub(super) fn article_index_at(
    article_spans: &[ArticleSpan],
    page_index: usize,
    line_index: usize,
) -> Option<usize> {
    if article_spans.is_empty() {
        return Some(0);
    }
    let coordinate = (page_index, line_index);
    article_spans
        .iter()
        .find(|span| {
            coordinate >= (span.start_page_index, span.start_line_index)
                && coordinate < (span.end_page_index, span.end_line_index)
        })
        .map(|span| span.article_index)
}

/// Positions of the longest strictly ascending subsequence of `numbers`.
pub(super) fn longest_ascending_run(numbers: &[u16]) -> HashSet<usize> {
    if numbers.is_empty() {
        return HashSet::new();
    }
    let mut best = vec![1usize; numbers.len()];
    let mut previous = vec![usize::MAX; numbers.len()];
    let mut end = 0usize;
    for i in 0..numbers.len() {
        for j in 0..i {
            if numbers[j] < numbers[i] && best[j] + 1 > best[i] {
                best[i] = best[j] + 1;
                previous[i] = j;
            }
        }
        if best[i] > best[end] {
            end = i;
        }
    }
    let mut run = HashSet::new();
    let mut cursor = end;
    while cursor != usize::MAX {
        run.insert(cursor);
        cursor = previous[cursor];
    }
    run
}
