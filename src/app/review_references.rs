//! Conservative links between numbered Review notes.
use super::*;
use std::ops::Range;

#[derive(Clone, Debug)]
struct Target {
    block: usize,
    body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthorityKind {
    Case,
    FederalStatute,
    LawReviewArticle,
}

#[derive(Default)]
pub(super) struct ReferenceIndex {
    unique_notes: HashSet<u16>,
    targets: HashMap<u16, Target>,
    /// Source note -> (previous note, exact source body). Only populated for
    /// narrowly recognizable, single-authority citations.
    id_targets: HashMap<u16, (u16, String, AuthorityKind)>,
}

fn has_callout(text: &str, target: u16) -> bool {
    let mut rest = text;
    while let Some((_, after)) = rest.split_once(layout_roles::CALLOUT_START) {
        let Some((marker, after_marker)) = after.split_once(layout_roles::CALLOUT_END) else {
            break;
        };
        if marker.trim().parse::<u16>().ok() == Some(target) {
            return true;
        }
        rest = after_marker;
    }
    false
}

impl ReferenceIndex {
    pub(super) fn new(document: &LiquidDocument) -> Self {
        let mut notes: HashMap<u16, Option<(usize, String)>> = HashMap::new();
        let mut note_order = Vec::new();
        for (block_index, block) in document
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| is_review_note_display_block(block))
        {
            for (marker, body) in split_fused_review_notes(&block.text) {
                let Ok(number) = marker.parse::<u16>() else {
                    continue;
                };
                if number == 0 || body.trim().is_empty() {
                    continue;
                }
                note_order.push(number);
                notes
                    .entry(number)
                    .and_modify(|entry| *entry = None)
                    .or_insert_with(|| Some((block_index, body.trim().to_owned())));
            }
        }
        let unique_notes = notes
            .iter()
            .filter_map(|(&number, note)| note.as_ref().map(|_| number))
            .collect();
        let mut link_counts: HashMap<u16, usize> = HashMap::new();
        for link in &document.footnote_links {
            *link_counts.entry(link.marker).or_default() += 1;
        }
        let mut targets = HashMap::new();
        for link in &document.footnote_links {
            if link_counts.get(&link.marker) != Some(&1) {
                continue;
            }
            let Some(Some((note_block, body))) = notes.get(&link.marker) else {
                continue;
            };
            let Some(block) = document.blocks.get(link.body_block_index) else {
                continue;
            };
            if *note_block != link.note_block_index
                || is_review_note_display_block(block)
                || !has_callout(&block.text, link.marker)
            {
                continue;
            }
            let target = Target {
                block: link.body_block_index,
                body: body.clone(),
            };
            targets.insert(link.marker, target);
        }
        let mut id_targets = HashMap::new();
        for pair in note_order.windows(2) {
            let [previous, source] = [pair[0], pair[1]];
            if previous.checked_add(1) != Some(source) || !targets.contains_key(&previous) {
                continue;
            }
            let (Some(Some((_, previous_body))), Some(Some((_, source_body)))) =
                (notes.get(&previous), notes.get(&source))
            else {
                continue;
            };
            let inherited = id_targets.get(&previous).and_then(
                |(_, expected, kind): &(u16, String, AuthorityKind)| {
                    (expected == previous_body).then_some(*kind)
                },
            );
            if let Some(kind) = authority_kind(previous_body).or(inherited)
                && standalone_id_range(source_body, kind).is_some()
            {
                id_targets.insert(source, (previous, source_body.clone(), kind));
            }
        }
        Self {
            unique_notes,
            targets,
            id_targets,
        }
    }

    fn references(&self, source: u16, text: &str) -> Vec<(Range<usize>, u16)> {
        if !self.unique_notes.contains(&source) {
            return Vec::new();
        }
        let mut references: Vec<_> = reference_ranges(text)
            .into_iter()
            .filter(|(_, target)| *target < source && self.targets.contains_key(target))
            .collect();
        if let Some((target, expected, kind)) = self.id_targets.get(&source)
            && text.trim() == expected.trim()
            && let Some(range) = standalone_id_range(text, *kind)
        {
            references.push((range, *target));
        }
        references.sort_by_key(|(range, _)| range.start);
        references
    }

    pub(super) fn append_to_job(
        &self,
        job: &mut LayoutJob,
        source: u16,
        text: &str,
        normal: TextFormat,
        accent: Color32,
        leading_space: f32,
    ) -> Vec<review_paragraph::MarkerRange> {
        let linked = TextFormat {
            color: accent,
            underline: Stroke::new(1.0_f32, accent),
            ..normal.clone()
        };
        let offset = job.text.chars().count();
        let mut gap = leading_space;
        let mut last = 0;
        let mut markers = Vec::new();
        for (range, number) in self.references(source, text) {
            if range.start > last {
                job.append(&text[last..range.start], gap, normal.clone());
                gap = 0.0;
            }
            job.append(&text[range.clone()], gap, linked.clone());
            gap = 0.0;
            markers.push(review_paragraph::MarkerRange {
                start: offset + text[..range.start].chars().count(),
                end: offset + text[..range.end].chars().count(),
                number,
            });
            last = range.end;
        }
        if last < text.len() {
            job.append(&text[last..], gap, normal);
        }
        markers
    }
}

fn numeric_pin(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let mut pieces = text.split(['-', '–']);
    let first = pieces.next().unwrap_or_default();
    let valid = |piece: &str| !piece.is_empty() && piece.bytes().all(|byte| byte.is_ascii_digit());
    valid(first) && pieces.next().is_none_or(valid) && pieces.next().is_none()
}

/// Match only an entire note consisting of Id. and a supported pinpoint.
/// A section pinpoint is accepted only for an already-recognized statute.
fn standalone_id_range(text: &str, kind: AuthorityKind) -> Option<Range<usize>> {
    let trimmed = text.trim();
    let prefix = trimmed.get(..3)?;
    if !prefix.eq_ignore_ascii_case("id.") {
        return None;
    }
    let rest = &trimmed[3..];
    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let suffix = rest.trim();
    if !suffix.is_empty() {
        let valid_pin = if let Some(section) = suffix.strip_prefix("§ ") {
            kind == AuthorityKind::FederalStatute
                && section_designator(section.strip_suffix('.').unwrap_or(section))
        } else if suffix
            .get(..3)
            .is_some_and(|at| at.eq_ignore_ascii_case("at "))
        {
            let pin = suffix[3..].strip_suffix('.').unwrap_or(&suffix[3..]);
            numeric_pin(pin)
        } else {
            false
        };
        if !valid_pin {
            return None;
        }
    }
    let start = text.len() - text.trim_start().len();
    Some(start..start + 3)
}

/// Require a trailing four-digit year and final period.
fn citation_before_year(text: &str) -> Option<&str> {
    let without_period = text.trim().strip_suffix('.')?;
    let (citation, parenthetical) = without_period.rsplit_once(" (")?;
    let year = parenthetical.strip_suffix(')')?;
    (year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit())).then_some(citation)
}

/// Recognize only simple, single-authority cases, federal statutes, or law
/// review articles. Other citation forms are intentionally left unlinked.
fn authority_kind(text: &str) -> Option<AuthorityKind> {
    let citation = citation_before_year(text)?;
    if single_case_citation(citation) {
        Some(AuthorityKind::Case)
    } else if single_federal_statute(citation) {
        Some(AuthorityKind::FederalStatute)
    } else if single_law_review_article(citation) {
        Some(AuthorityKind::LawReviewArticle)
    } else {
        None
    }
}

fn single_case_citation(citation: &str) -> bool {
    let mut fields = citation.split(',').map(str::trim);
    let (Some(case), Some(reporter)) = (fields.next(), fields.next()) else {
        return false;
    };
    let pin = fields.next();
    if fields.next().is_some() || pin.is_some_and(|pin| !numeric_pin(pin)) {
        return false;
    }
    let Some((left, right)) = case.split_once(" v. ") else {
        return false;
    };
    let valid_party = |party: &str| {
        !party.trim().is_empty()
            && party.chars().all(|ch| {
                ch.is_alphanumeric() || ch.is_whitespace() || matches!(ch, '&' | '\'' | '’' | '-')
            })
    };
    if !valid_party(left) || !valid_party(right) {
        return false;
    }
    let mut parts = reporter.split_whitespace();
    let (Some(volume), Some(series), Some(page)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    parts.next().is_none()
        && matches!(series, "U.S." | "F.2d" | "F.3d" | "F.4th")
        && numeric_pin(volume)
        && !volume.contains(['-', '–'])
        && numeric_pin(page)
        && !page.contains(['-', '–'])
}

fn single_federal_statute(citation: &str) -> bool {
    let Some((title, section)) = citation.split_once(" U.S.C. § ") else {
        return false;
    };
    !title.is_empty()
        && title.bytes().all(|byte| byte.is_ascii_digit())
        && section_designator(section)
}

fn section_designator(section: &str) -> bool {
    if !section
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_digit())
        || section.contains("()")
    {
        return false;
    }
    let mut depth = 0u32;
    for ch in section.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ch if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' => {}
            _ => return false,
        }
    }
    depth == 0
        && section
            .chars()
            .last()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == ')')
}

fn single_law_review_article(citation: &str) -> bool {
    let fields = citation.split(',').map(str::trim).collect::<Vec<_>>();
    if !(3..=4).contains(&fields.len()) || fields.get(3).is_some_and(|pin| !numeric_pin(pin)) {
        return false;
    }
    let (author, title, reporter) = (fields[0], fields[1], fields[2]);
    if author.split_whitespace().count() < 2
        || !author.chars().all(|ch| {
            ch.is_alphabetic() || ch.is_whitespace() || matches!(ch, '.' | '\'' | '’' | '-' | '&')
        })
        || title.chars().filter(|ch| ch.is_alphabetic()).count() < 4
        || !title.chars().all(|ch| {
            ch.is_alphanumeric()
                || ch.is_whitespace()
                || matches!(ch, ':' | '-' | '–' | '\'' | '’' | '?' | '!')
        })
    {
        return false;
    }
    let parts = reporter.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 4 {
        return false;
    }
    let volume = parts[0];
    let page = parts[parts.len() - 1];
    let series = parts[1..parts.len() - 1].join(" ");
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    digits(volume)
        && digits(page)
        && (series.ends_with(" L. Rev.") || series.ends_with(" L.J."))
        && series
            .chars()
            .all(|ch| ch.is_alphabetic() || ch.is_whitespace() || matches!(ch, '.' | '-' | '&'))
}

/// Byte ranges preserve the original Unicode text; only the ASCII legal phrase
/// is matched case-insensitively. Never resolve a plural or malformed number.
fn reference_ranges(text: &str) -> Vec<(Range<usize>, u16)> {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for (start, _) in lower.match_indices("supra") {
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        let mut at = start + 5;
        let before_space = at;
        while let Some(ch) = text[at..].chars().next().filter(|ch| ch.is_whitespace()) {
            at += ch.len_utf8();
        }
        if at == before_space || !lower[at..].starts_with("note") {
            continue;
        }
        at += 4;
        let before_space = at;
        while let Some(ch) = text[at..].chars().next().filter(|ch| ch.is_whitespace()) {
            at += ch.len_utf8();
        }
        if at == before_space {
            continue;
        }
        let digits = at;
        while at < text.len() && text.as_bytes()[at].is_ascii_digit() {
            at += 1;
        }
        if digits == at
            || text[at..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        if text[at..].starts_with(['-', '–', '—'])
            || (text[at..].starts_with('.')
                && text[at + 1..].starts_with(|ch: char| ch.is_ascii_digit()))
        {
            continue;
        }
        if let Ok(number) = text[digits..at].parse::<u16>()
            && number > 0
        {
            ranges.push((start..at, number));
        }
    }
    ranges
}

impl PdfEditorApp {
    pub(super) fn draw_note_reference_text(
        &mut self,
        ui: &mut egui::Ui,
        source: u16,
        text: &str,
        size: f32,
        color: Color32,
        accent: Color32,
    ) {
        let references = self.review_references.references(source, text);
        if references.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(text).size(size).color(color))
                    .wrap()
                    .selectable(true),
            );
            return;
        }
        let normal = TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        };
        let mut job = LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        let markers = self
            .review_references
            .append_to_job(&mut job, source, text, normal, accent, 0.0);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let response = ui.add(egui::Label::new(Arc::clone(&galley)).selectable(true));
        self.interact_note_references(ui, source, &galley, response.rect.min, &markers);
    }

    pub(super) fn interact_note_references(
        &mut self,
        ui: &mut egui::Ui,
        source: u16,
        galley: &egui::Galley,
        origin: Pos2,
        markers: &[review_paragraph::MarkerRange],
    ) -> bool {
        let mut selected = None;
        for (ordinal, (rect, number)) in review_paragraph::marker_rects(galley, origin, markers)
            .into_iter()
            .enumerate()
        {
            let rect = rect.intersect(ui.clip_rect());
            if !rect.is_positive() {
                continue;
            }
            let Some(target) = self.review_references.targets.get(&number) else {
                continue;
            };
            let preview = target.body.chars().take(800).collect::<String>();
            let link = ui
                .interact(
                    rect,
                    ui.id().with(("note-reference", source, ordinal)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(format!("Note {number}\n{preview}"));
            link.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Go to note {number}"),
                )
            });
            if link.clicked() {
                selected = Some((target.block, number));
            }
        }
        if let Some((block, number)) = selected {
            self.jump_to_review_block(block);
            self.chrome.expanded_note = None;
            self.pending_reference_note = Some((block, number));
            self.status = format!("Jumping to note {number}. Use Alt+Left to return.");
            ui.ctx().request_repaint();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::LiquidFootnoteLink;

    fn fixture() -> Arc<LiquidDocument> {
        let first = format!(
            "First passage {}1{}.",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        let second = format!(
            "Second passage {}2{}.",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "title":"Article", "blocks":[
                    {"role":"paragraph","text":first},
                    {"role":"footnote","text":"1. Earlier authority."},
                    {"role":"paragraph","text":second},
                    {"role":"footnote","text":"2. See supra note 1, at 42."}
                ], "footnote_links":[
                    {"body_block_index":0,"body_marker_ordinal":0,"marker":1,"note_block_index":1},
                    {"body_block_index":2,"body_marker_ordinal":0,"marker":2,"note_block_index":3}
                ], "llm_used":false,"warnings":[],"source_signature":"references-test"
            }))
            .unwrap(),
        )
    }

    #[test]
    fn matches_singular_references_and_preserves_unicode_offsets() {
        let text = "Café SUPRA note 17, supra note 2. asupra note 3; supra notes 4; supra note 9a; supra note 1-2; supra note 1.3.";
        let ranges = reference_ranges(text);
        assert_eq!(
            ranges
                .iter()
                .map(|(range, number)| (&text[range.clone()], *number))
                .collect::<Vec<_>>(),
            vec![("SUPRA note 17", 17), ("supra note 2", 2)]
        );
    }

    #[test]
    fn recognizes_only_standalone_id_and_a_simple_single_case() {
        for text in ["Id.", " id. at 42. ", "Id. at 42–45."] {
            let range = standalone_id_range(text, AuthorityKind::Case).expect(text);
            assert_eq!(text[range].to_ascii_lowercase(), "id.");
        }
        for text in [
            "Id. at 42; Smith v. Jones.",
            "Id.at 42.",
            "Id. at 42 n.3.",
            "See Id.",
            "Id. at ",
        ] {
            assert!(
                standalone_id_range(text, AuthorityKind::Case).is_none(),
                "{text}"
            );
        }
        for text in [
            "Smith v. Jones, 410 U.S. 113 (1973).",
            "Smith v. Jones, 410 U.S. 113, 115–16 (1973).",
            "Smith v. Jones, 123 F.3d 456 (1999).",
        ] {
            assert!(authority_kind(text).is_some(), "{text}");
        }
        for text in [
            "Smith v. Jones, 410 U.S. 113 (1973); Adams v. Baker, 20 F.3d 12 (1991).",
            "Smith v. Jones, 410 U.S. 113 (1973) (quoting another source).",
            "Smith v. Jones, 410 U.S. 113 (1973). See also Adams.",
            "Smith v. Jones, 410 U.S. 113, 115, 116 (1973).",
            "Some discussion without a citation.",
            "Id. at 42.",
        ] {
            assert!(authority_kind(text).is_none(), "{text}");
        }
    }

    #[test]
    fn recognizes_simple_statutes_and_law_review_articles_without_guessing() {
        let accepted = [
            ("42 U.S.C. § 1983 (2018).", AuthorityKind::FederalStatute),
            (
                "15 U.S.C. § 78j(b)(1) (2018).",
                AuthorityKind::FederalStatute,
            ),
            (
                "Jane Smith, Contracts and Algorithms, 123 Harv. L. Rev. 456, 460 (2020).",
                AuthorityKind::LawReviewArticle,
            ),
            (
                "Jane Smith, Legal Theory, 35 Yale L.J. 10 (2021).",
                AuthorityKind::LawReviewArticle,
            ),
        ];
        for (citation, expected) in accepted {
            assert_eq!(authority_kind(citation), Some(expected), "{citation}");
            let mut document = (*fixture()).clone();
            document.blocks[1].text = format!("1. {citation}");
            document.blocks[3].text = "2. Id.".into();
            assert_eq!(
                ReferenceIndex::new(&document).references(2, "Id."),
                vec![(0..3, 1)]
            );
        }
        for citation in [
            "42 U.S.C. § 1983 (2018); 15 U.S.C. § 78j (2018).",
            "42 U.S.C. §§ 1983 (2018).",
            "42 U.S.C. § 1983(a (2018).",
            "Jane Smith, Legal Theory, 123 Harv. L. Rev. 456 (2020) (quoting another source).",
            "Jane Smith, Legal Theory, 123 Harv. L. Rev. 456; Doe v. Roe, 1 U.S. 2 (2000).",
            "Jane Smith, Legal Theory, 123 Unknown Reporter 456 (2020).",
        ] {
            assert!(authority_kind(citation).is_none(), "{citation}");
        }
        assert_eq!(
            standalone_id_range("Id. § 1985(a).", AuthorityKind::FederalStatute),
            Some(0..3)
        );
        assert!(standalone_id_range("Id. § 1985(a).", AuthorityKind::Case).is_none());
        assert!(standalone_id_range("Id. § 1985(a.", AuthorityKind::FederalStatute).is_none());
        let mut document = (*fixture()).clone();
        document.blocks[1].text = "1. 42 U.S.C. § 1983 (2018).".into();
        document.blocks[3].text = "2. Id. § 1985(a).".into();
        assert_eq!(
            ReferenceIndex::new(&document).references(2, "Id. § 1985(a)."),
            vec![(0..3, 1)]
        );
        let mut body = document.blocks[2].clone();
        body.text = format!(
            "Third passage {}3{}.",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        let mut note = document.blocks[3].clone();
        note.text = "3. Id. § 1986.".into();
        document.blocks.push(body);
        document.blocks.push(note);
        document.footnote_links.push(LiquidFootnoteLink {
            body_block_index: 4,
            body_marker_ordinal: 0,
            marker: 3,
            note_block_index: 5,
            body_page_index: None,
            note_page_index: None,
        });
        assert_eq!(
            ReferenceIndex::new(&document).references(3, "Id. § 1986."),
            vec![(0..3, 2)]
        );
        document.blocks[1].text = "1. Smith v. Jones, 410 U.S. 113 (1973).".into();
        assert!(
            ReferenceIndex::new(&document)
                .references(2, "Id. § 1985(a).")
                .is_empty()
        );
        assert!(
            ReferenceIndex::new(&document)
                .references(3, "Id. § 1986.")
                .is_empty()
        );
    }

    #[test]
    fn id_links_only_when_the_adjacent_unique_note_has_one_recognizable_case() {
        let mut document = (*fixture()).clone();
        document.blocks[1].text = "1. Smith v. Jones, 410 U.S. 113, 115 (1973).".into();
        document.blocks[3].text = "2. Id. at 116.".into();
        let index = ReferenceIndex::new(&document);
        assert_eq!(index.references(2, "Id. at 116."), vec![(0..3, 1)]);
        assert!(index.references(2, "Id. at 117.").is_empty());
        assert!(index.references(1, "Id. at 116.").is_empty());

        let mut ambiguous = document.clone();
        ambiguous.blocks[1].text =
            "1. Smith v. Jones, 410 U.S. 113 (1973); Adams v. Baker, 20 F.3d 12 (1991).".into();
        assert!(
            ReferenceIndex::new(&ambiguous)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut mixed = document.clone();
        mixed.blocks[3].text = "2. Id. at 116; see also Adams.".into();
        assert!(
            ReferenceIndex::new(&mixed)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut duplicate = document.clone();
        duplicate.blocks.push(duplicate.blocks[1].clone());
        assert!(
            ReferenceIndex::new(&duplicate)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut duplicate_source = document.clone();
        duplicate_source
            .blocks
            .push(duplicate_source.blocks[3].clone());
        assert!(
            ReferenceIndex::new(&duplicate_source)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut duplicate_link = document.clone();
        duplicate_link
            .footnote_links
            .push(duplicate_link.footnote_links[0].clone());
        assert!(
            ReferenceIndex::new(&duplicate_link)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut missing_link = document.clone();
        missing_link.footnote_links.retain(|link| link.marker != 1);
        assert!(
            ReferenceIndex::new(&missing_link)
                .references(2, "Id. at 116.")
                .is_empty()
        );

        let mut out_of_order = document.clone();
        let mut unrelated = out_of_order.blocks[1].clone();
        unrelated.text = "3. An intervening note.".into();
        out_of_order.blocks.insert(3, unrelated);
        out_of_order.footnote_links[1].note_block_index = 4;
        assert!(
            ReferenceIndex::new(&out_of_order)
                .references(2, "Id. at 116.")
                .is_empty()
        );
    }

    #[test]
    fn consecutive_id_notes_link_to_the_immediate_note_and_keep_its_pinpoint() {
        let mut document = (*fixture()).clone();
        document.blocks[1].text = "1. Smith v. Jones, 410 U.S. 113, 115 (1973).".into();
        document.blocks[3].text = "2. Id. at 116.".into();
        let mut third_body = document.blocks[2].clone();
        third_body.text = format!(
            "Third passage {}3{}.",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        let mut third_note = document.blocks[3].clone();
        third_note.text = "3. Id.".into();
        document.blocks.push(third_body);
        document.blocks.push(third_note);
        document.footnote_links.push(LiquidFootnoteLink {
            body_block_index: 4,
            body_marker_ordinal: 0,
            marker: 3,
            note_block_index: 5,
            body_page_index: None,
            note_page_index: None,
        });
        let index = ReferenceIndex::new(&document);
        assert_eq!(index.references(2, "Id. at 116."), vec![(0..3, 1)]);
        assert_eq!(index.references(3, "Id."), vec![(0..3, 2)]);
        assert_eq!(index.targets[&2].body, "Id. at 116.");

        let mut ambiguous = document.clone();
        ambiguous.blocks[3].text = "2. Id. at 116; see also Adams.".into();
        assert!(
            ReferenceIndex::new(&ambiguous)
                .references(3, "Id.")
                .is_empty()
        );
        let mut duplicate = document.clone();
        duplicate.blocks.push(duplicate.blocks[3].clone());
        assert!(
            ReferenceIndex::new(&duplicate)
                .references(3, "Id.")
                .is_empty()
        );
    }

    #[test]
    fn id_reference_exposes_an_accessible_jump_to_the_preceding_note() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut doc = (*fixture()).clone();
        doc.blocks[1].text = "1. Smith v. Jones, 410 U.S. 113 (1973).".into();
        doc.blocks[3].text = "2. Id. at 116.".into();
        let document = Arc::new(doc);
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::clone(&document));
        app.refresh_review_derived(&document);
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_note_reference_text(
                    ui,
                    2,
                    "Id. at 116.",
                    15.0,
                    Color32::BLACK,
                    Color32::DARK_RED,
                )
            });
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        let id = tree
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some("Go to note 1")).then_some(*id))
            .expect("accessible Id. reference");
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target: id,
                    data: None,
                },
            )],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_note_reference_text(
                    ui,
                    2,
                    "Id. at 116.",
                    15.0,
                    Color32::BLACK,
                    Color32::DARK_RED,
                )
            });
        });
        assert_eq!(app.liquid_scroll_to_block, Some(0));
        assert_eq!(app.pending_reference_note, Some((0, 1)));
    }

    #[test]
    fn refuses_duplicate_notes_duplicate_links_and_forward_references() {
        let mut document = (*fixture()).clone();
        let index = ReferenceIndex::new(&document);
        assert_eq!(index.references(2, "See supra note 1").len(), 1);
        assert!(index.references(1, "See supra note 2").is_empty());
        document.blocks.push(document.blocks[1].clone());
        assert!(
            ReferenceIndex::new(&document)
                .references(2, "supra note 1")
                .is_empty()
        );
        document.blocks.pop();
        document.footnote_links.push(LiquidFootnoteLink {
            body_block_index: 0,
            body_marker_ordinal: 0,
            marker: 1,
            note_block_index: 1,
            body_page_index: None,
            note_page_index: None,
        });
        assert!(
            ReferenceIndex::new(&document)
                .references(2, "supra note 1")
                .is_empty()
        );
    }

    #[test]
    fn accessible_reference_click_records_jump_and_opens_the_target_on_arrival() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = fixture();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::clone(&document));
        app.refresh_review_derived(&document);
        app.search_state.review_anchor = Some(2);
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_note_reference_text(
                    ui,
                    2,
                    "See supra note 1, at 42.",
                    15.0,
                    Color32::BLACK,
                    Color32::DARK_RED,
                )
            });
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        let id = tree
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some("Go to note 1")).then_some(*id))
            .expect("accessible reference");
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target: id,
                    data: None,
                },
            )],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_note_reference_text(
                    ui,
                    2,
                    "See supra note 1, at 42.",
                    15.0,
                    Color32::BLACK,
                    Color32::DARK_RED,
                )
            });
        });
        assert_eq!(app.liquid_scroll_to_block, Some(0));
        assert_eq!(app.pending_reference_note, Some((0, 1)));
        let popup_id = liquid_footnote_popup_id(
            &liquid_feedback_id(&document.source_signature, 0, &document.blocks[0]),
            1,
        );
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.draw_liquid_feedback_block(ui, &document, 0, &document.blocks[0]);
                });
            },
        );
        assert!(app.pending_reference_note.is_none());
        assert!(egui::Popup::is_id_open(&ctx, popup_id));
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.liquid_scroll_to_block, Some(2));
    }

    #[test]
    fn pointer_click_on_reference_jumps_to_the_earlier_note() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = fixture();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::clone(&document));
        app.refresh_review_derived(&document);
        let body = "See supra note 1, at 42.";
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.draw_note_reference_text(
                            ui,
                            2,
                            body,
                            15.0,
                            Color32::BLACK,
                            Color32::DARK_RED,
                        )
                    });
                },
            )
        };
        let output = draw(&mut app, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.text() == body {
                        return review_paragraph::marker_rects(
                            &text.galley,
                            text.pos,
                            &[review_paragraph::MarkerRange {
                                start: 4,
                                end: 16,
                                number: 1,
                            }],
                        )
                        .first()
                        .map(|(rect, _)| rect.center());
                    }
                }
                None
            })
            .expect("reference geometry");
        let hovered = draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        assert_eq!(
            hovered.platform_output.cursor_icon,
            CursorIcon::PointingHand
        );
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            let _ = draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.pending_reference_note, Some((0, 1)));
    }

    #[test]
    fn unshortened_margin_note_reference_is_clickable() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = fixture();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::clone(&document));
        app.refresh_review_derived(&document);
        let mut collector = ReviewMarginCollector::default();
        collector.add_note_block(3, &document.blocks[3].text, 80.0);
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.draw_review_margin_notes(ui, collector.clone(), 300.0, 0.0, 280.0)
                    });
                },
            )
        };
        let output = draw(&mut app, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    let visible = text.galley.text();
                    if let Some((range, number)) = reference_ranges(visible).first() {
                        let marker = review_paragraph::MarkerRange {
                            start: visible[..range.start].chars().count(),
                            end: visible[..range.end].chars().count(),
                            number: *number,
                        };
                        return review_paragraph::marker_rects(&text.galley, text.pos, &[marker])
                            .first()
                            .map(|(rect, _)| rect.center());
                    }
                }
                None
            })
            .expect("margin reference geometry");
        let _ = draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            let _ = draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.pending_reference_note, Some((0, 1)));
        assert!(app.chrome.expanded_note.is_none());
    }
}
