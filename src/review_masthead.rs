//! The document identity shown in the masthead: authors, title, and citation.
//!
//! Law review PDFs announce who and where they are in a few predictable places:
//! the byline on the first page, the running heads (`452 NEW YORK UNIVERSITY LAW
//! REVIEW [Vol. 99:451` and `May 2024] GENERATIVE INTERPRETATION 453`), and the
//! volume line on the opening page. The masthead reads those lines from the
//! PDF's own text, so it works in the original view before Review Mode has run.
//! When Review Mode has a title, that title wins. Everything here is text in,
//! text out, and has no view state.

use crate::layout_roles::{CALLOUT_END, CALLOUT_START};
use crate::liquid::{LiquidBlock, LiquidBlockRole};

/// What the masthead prints. Empty strings mean "nothing found"; the caller
/// decides what to show instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MastheadIdentity {
    pub title: String,
    pub authors: String,
    pub citation: String,
}

/// How many opening pages to read. Running heads start on the second page.
pub const MASTHEAD_PAGES_TO_READ: usize = 5;

/// Build the identity from the opening pages' text (page order, possibly
/// incomplete), an optional Review Mode title and blocks, and a fallback title
/// (usually the file name).
pub fn masthead_identity(
    page_texts: &[&str],
    review_title: Option<&str>,
    review_blocks: &[LiquidBlock],
    fallback_title: &str,
) -> MastheadIdentity {
    let heads = RunningHeads::read(page_texts);
    let opening = page_texts.first().map(|text| opening_page(text));

    // The printed opening page is the most reliable witness. Review Mode's
    // title comes next, unless it picked up the journal's name instead.
    let journal_name = heads
        .journal
        .clone()
        .or_else(|| opening.as_ref().and_then(|page| page.journal.clone()));
    let title = opening
        .as_ref()
        .map(|page| page.title.clone())
        .filter(|title| plausible_title(title))
        .or_else(|| {
            review_title
                .map(clean_display_text)
                .filter(|title| plausible_title(title) && !looks_like_masthead_noise(title))
                .filter(|title| {
                    journal_name
                        .as_deref()
                        .is_none_or(|journal| !journal.eq_ignore_ascii_case(title))
                })
        })
        .or_else(|| heads.short_title.clone())
        .unwrap_or_else(|| clean_display_text(fallback_title));

    let authors = opening
        .as_ref()
        .map(|page| page.byline.clone())
        .filter(|byline| !byline.is_empty())
        .or_else(|| review_byline(review_blocks))
        .unwrap_or_default();

    let journal = journal_name;
    let volume = heads
        .volume
        .or_else(|| opening.as_ref().and_then(|page| page.volume));
    let first_page = heads
        .first_page
        .or_else(|| opening.as_ref().and_then(|page| page.folio));
    let year = heads
        .year
        .or_else(|| opening.as_ref().and_then(|page| page.year));

    MastheadIdentity {
        title,
        authors,
        citation: format_citation(journal.as_deref(), volume, first_page, year),
    }
}

/// `99 New York University Law Review 451 · 2024`, degrading gracefully.
pub fn format_citation(
    journal: Option<&str>,
    volume: Option<u32>,
    first_page: Option<u32>,
    year: Option<u32>,
) -> String {
    let Some(journal) = journal.map(str::trim).filter(|journal| !journal.is_empty()) else {
        return year.map(|year| year.to_string()).unwrap_or_default();
    };
    let mut citation = String::new();
    if let Some(volume) = volume {
        citation.push_str(&volume.to_string());
        citation.push(' ');
    }
    citation.push_str(&title_case_journal(journal));
    if let (Some(_), Some(first_page)) = (volume, first_page) {
        citation.push(' ');
        citation.push_str(&first_page.to_string());
    }
    if let Some(year) = year {
        citation.push_str(" · ");
        citation.push_str(&year.to_string());
    }
    citation
}

#[derive(Debug, Default)]
struct RunningHeads {
    journal: Option<String>,
    volume: Option<u32>,
    first_page: Option<u32>,
    year: Option<u32>,
    short_title: Option<String>,
}

impl RunningHeads {
    fn read(page_texts: &[&str]) -> Self {
        let mut heads = Self::default();
        for text in page_texts.iter().skip(1).take(MASTHEAD_PAGES_TO_READ) {
            let lines = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            let edge = lines
                .iter()
                .take(3)
                .chain(lines.iter().rev().take(2))
                .copied();
            for line in edge {
                if let Some((journal, volume, first_page)) = journal_volume_head(line) {
                    heads.journal.get_or_insert(journal);
                    heads.volume.get_or_insert(volume);
                    heads.first_page.get_or_insert(first_page);
                } else if let Some((year, title)) = dated_title_head(line) {
                    heads.year.get_or_insert(year);
                    if plausible_title(&title) {
                        heads.short_title.get_or_insert(title);
                    }
                }
            }
        }
        heads
    }
}

/// `452 NEW YORK UNIVERSITY LAW REVIEW [Vol. 99:451` -> journal, volume, first page.
fn journal_volume_head(line: &str) -> Option<(String, u32, u32)> {
    // ASCII-only case folding keeps byte offsets valid for slicing `line`.
    let upper = line.to_ascii_uppercase();
    let at = upper.find("VOL.")?;
    let (volume, rest) = leading_number(upper[at + 4..].trim_start())?;
    let rest = rest.strip_prefix(':')?;
    let (first_page, _) = leading_number(rest)?;
    let before = line[..at].trim().trim_end_matches('[').trim();
    let journal = strip_folio(before);
    looks_like_journal(&journal).then_some((journal, volume, first_page))
}

/// `May 2024] GENERATIVE INTERPRETATION 453` -> year and short title.
fn dated_title_head(line: &str) -> Option<(u32, String)> {
    let close = line.find(']')?;
    let (date, rest) = line.split_at(close);
    let tokens = date.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > 3 {
        return None;
    }
    let mut year = None;
    for token in tokens {
        let token = token.trim_matches(|ch: char| matches!(ch, '[' | ',' | '.'));
        match token.parse::<u32>() {
            Ok(value) if (1800..=2100).contains(&value) => year = Some(value),
            _ if is_month_word(token) => {}
            _ => return None,
        }
    }
    let year = year?;
    let title = strip_trailing_folio(rest.trim_start_matches(']').trim());
    let title = clean_display_text(&title);
    (!title.is_empty()).then_some((year, title))
}

fn is_month_word(token: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let upper = token.to_ascii_uppercase();
    upper.len() >= 3 && MONTHS.iter().any(|month| upper.starts_with(month))
}

#[derive(Debug, Default)]
struct OpeningPage {
    title: String,
    byline: String,
    journal: Option<String>,
    volume: Option<u32>,
    year: Option<u32>,
    folio: Option<u32>,
}

/// Read the first page: folio, journal name and volume line, the title block,
/// and the byline that follows it.
fn opening_page(text: &str) -> OpeningPage {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(40)
        .collect::<Vec<_>>();
    let mut page = OpeningPage::default();
    if let Some(first) = lines.first() {
        page.folio = first.parse::<u32>().ok();
    }

    // Journal name: consecutive capitalised lines near the top that end in a
    // journal word, possibly split across two lines ("NEW YORK UNIVERSITY" / "LAW REVIEW").
    for window_len in [2usize, 1] {
        if page.journal.is_some() {
            break;
        }
        for window in lines.iter().take(8).collect::<Vec<_>>().windows(window_len) {
            let joined = window
                .iter()
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join(" ");
            let candidate = strip_folio(&joined);
            if looks_like_journal(&candidate) && candidate.len() <= 80 {
                page.journal = Some(candidate);
                break;
            }
        }
    }

    for line in lines.iter().take(10) {
        let upper = line.to_ascii_uppercase();
        if let Some(at) = upper.find("VOLUME") {
            if let Some((volume, _)) = leading_number(upper[at + 6..].trim_start()) {
                page.volume.get_or_insert(volume);
            }
            if let Some(year) = upper
                .split_whitespace()
                .filter_map(|token| token.parse::<u32>().ok())
                .find(|year| (1800..=2100).contains(year))
            {
                page.year.get_or_insert(year);
            }
        }
    }

    let byline_at = lines
        .iter()
        .position(|line| looks_like_byline(line))
        .filter(|at| *at > 0);
    if let Some(at) = byline_at {
        page.byline = clean_byline(lines[at]);
        let mut title_lines = Vec::new();
        for line in lines[..at].iter().rev() {
            if !is_title_line(line) || title_lines.len() >= 4 {
                break;
            }
            title_lines.push(*line);
        }
        title_lines.reverse();
        page.title = clean_display_text(&title_lines.join(" "));
    } else if let Some(label_at) = lines
        .iter()
        .take(12)
        .position(|line| is_section_label(line))
    {
        // Unmarked bylines: "ARTICLE" / title lines / the author's name.
        let title_lines = lines[label_at + 1..]
            .iter()
            .take_while(|line| is_title_line(line))
            .take(4)
            .copied()
            .collect::<Vec<_>>();
        if !title_lines.is_empty() {
            page.title = clean_display_text(&title_lines.join(" "));
            if let Some(next) = lines.get(label_at + 1 + title_lines.len()) {
                let names = split_names(&clean_byline(next));
                if !names.is_empty()
                    && names.len() <= 6
                    && names.iter().all(|name| looks_like_person_name(name))
                {
                    page.byline = clean_byline(next);
                }
            }
        }
    }
    page
}

fn is_section_label(line: &str) -> bool {
    matches!(
        line.trim().to_ascii_uppercase().as_str(),
        "ARTICLE" | "ARTICLES" | "ESSAY" | "ESSAYS" | "NOTE" | "NOTES" | "COMMENT" | "COMMENTS"
    )
}

fn review_byline(blocks: &[LiquidBlock]) -> Option<String> {
    blocks
        .iter()
        .take(40)
        .filter(|block| {
            matches!(
                block.role,
                LiquidBlockRole::AuthorInfo | LiquidBlockRole::Metadata
            )
        })
        .map(|block| block.text.as_str())
        .find(|text| looks_like_byline(text))
        .map(clean_byline)
}

const NOTE_MARKS: [char; 8] = ['†', '‡', '*', '∗', '§', '¶', '‖', '#'];

fn looks_like_byline(line: &str) -> bool {
    let text = strip_callouts(line);
    let trimmed = text.trim();
    if trimmed.len() < 5 || trimmed.len() > 110 {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if [
        "professor",
        "university",
        "school",
        "law review",
        "volume",
        "copyright",
        "©",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return false;
    }
    let has_marks = trimmed.chars().any(|ch| NOTE_MARKS.contains(&ch));
    let names = split_names(&clean_byline(trimmed));
    if names.is_empty() || names.len() > 6 {
        return false;
    }
    if !names.iter().all(|name| looks_like_person_name(name)) {
        return false;
    }
    // A single bare name is only a byline when it carries an author-note mark.
    has_marks || names.len() > 1
}

fn split_names(byline: &str) -> Vec<String> {
    byline
        .replace(" & ", ",")
        .replace(" and ", ",")
        .replace(" AND ", ",")
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn looks_like_person_name(name: &str) -> bool {
    let tokens = name.split_whitespace().collect::<Vec<_>>();
    if !(2..=5).contains(&tokens.len()) {
        return false;
    }
    tokens.iter().all(|token| {
        let mut chars = token.chars();
        let first = chars.next();
        first.is_some_and(|ch| ch.is_uppercase())
            && token
                .chars()
                .all(|ch| ch.is_alphabetic() || matches!(ch, '.' | '\'' | '’' | '-'))
    }) || tokens
        .iter()
        .all(|token| ["Jr.", "Sr.", "II", "III"].contains(token))
}

fn clean_byline(line: &str) -> String {
    let text = strip_callouts(line);
    let cleaned = text
        .chars()
        .filter(|ch| !NOTE_MARKS.contains(ch) && !ch.is_ascii_digit())
        .collect::<String>();
    let collapsed = collapse_spaces(&cleaned)
        .replace(" and ", " & ")
        .replace(" AND ", " & ");
    collapsed
        .trim_matches(|ch: char| ch == ',' || ch.is_whitespace())
        .to_owned()
}

fn is_title_line(line: &str) -> bool {
    let letters = line.chars().filter(|ch| ch.is_alphabetic()).count();
    if letters < 3 || line.len() > 120 {
        return false;
    }
    let upper = line.to_uppercase();
    if is_section_label(line)
        || upper.starts_with("VOLUME")
        || upper.starts_with("CONTENTS")
        || upper.starts_with("INTRODUCTION")
        || upper.contains("LAW REVIEW ASSOCIATION")
        || upper.contains('©')
        || looks_like_journal(&strip_folio(line))
    {
        return false;
    }
    let uppercase = line.chars().filter(|ch| ch.is_uppercase()).count();
    uppercase * 10 >= letters * 7
}

/// A "title" that is really the journal, the institution, or a section label.
fn looks_like_masthead_noise(title: &str) -> bool {
    let upper = title.to_uppercase();
    looks_like_journal(title)
        || is_section_label(title)
        || matches!(
            upper.trim(),
            "NEW YORK UNIVERSITY" | "CONTENTS" | "INTRODUCTION"
        )
        || (upper.ends_with("UNIVERSITY") && upper.split_whitespace().count() <= 5)
        || upper.starts_with("UNIVERSITY OF")
}

fn plausible_title(title: &str) -> bool {
    let letters = title.chars().filter(|ch| ch.is_alphabetic()).count();
    letters >= 3 && title.len() <= 160
}

fn looks_like_journal(text: &str) -> bool {
    let upper = text.to_uppercase();
    let words = upper.split_whitespace().collect::<Vec<_>>();
    if words.len() < 2 || words.len() > 9 {
        return false;
    }
    let ends_in_journal_word = words.last().is_some_and(|last| {
        matches!(
            last.trim_end_matches(['.', ',']),
            "REVIEW" | "JOURNAL" | "REV" | "L.J" | "QUARTERLY" | "FORUM" | "BULLETIN"
        )
    });
    if matches!(
        upper.trim(),
        "LAW REVIEW" | "LAW JOURNAL" | "THE LAW REVIEW"
    ) {
        return false;
    }
    let letters_only = text.chars().all(|ch| {
        ch.is_alphabetic() || ch.is_whitespace() || matches!(ch, '.' | '&' | '\'' | '’' | '-')
    });
    ends_in_journal_word && letters_only
}

fn title_case_journal(journal: &str) -> String {
    journal
        .split_whitespace()
        .enumerate()
        .map(|(index, word)| {
            let lower = word.to_lowercase();
            if index > 0 && matches!(lower.as_str(), "of" | "and" | "the" | "for" | "on" | "in") {
                lower
            } else if word.contains('.') && word.len() <= 5 {
                word.to_owned()
            } else {
                let mut chars = lower.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn leading_number(text: &str) -> Option<(u32, &str)> {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 6 {
        return None;
    }
    let value = text[..digits].parse().ok()?;
    Some((value, &text[digits..]))
}

fn strip_folio(text: &str) -> String {
    let trimmed = text.trim();
    let rest = trimmed.trim_start_matches(|ch: char| ch.is_ascii_digit());
    collapse_spaces(rest.trim())
}

fn strip_trailing_folio(text: &str) -> String {
    text.trim()
        .trim_end_matches(|ch: char| ch.is_ascii_digit())
        .trim()
        .to_owned()
}

fn strip_callouts(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_callout = false;
    for ch in text.chars() {
        match ch {
            CALLOUT_START => in_callout = true,
            CALLOUT_END => in_callout = false,
            _ if !in_callout => output.push(ch),
            _ => {}
        }
    }
    output
}

/// Remove callouts, note marks and stray spacing from a title-like string.
pub fn clean_display_text(text: &str) -> String {
    let text = strip_callouts(text);
    let cleaned = text
        .chars()
        .filter(|ch| !NOTE_MARKS.contains(ch))
        .collect::<String>();
    collapse_spaces(&cleaned)
        .trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | ';'))
        .to_owned()
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NYU_OPENING: &str = "451\r\nNEW YORK UNIVERSITY\r\nLAW REVIEW\r\nVolume 99 May 2024 Number 2\r\nARTICLES\r\nGENERATIVE INTERPRETATION\r\nYonathan Arbel† & David A. Hoffman‡\r\nWe introduce generative interpretation, a new approach to estimating contractual \r\nmeaning using large language models.";
    const NYU_EVEN: &str = "452 NEW YORK UNIVERSITY LAW REVIEW [Vol. 99:451\r\nINTRODUCTION\r\nIn 2023 a court was asked";
    const NYU_ODD: &str =
        "May 2024] GENERATIVE INTERPRETATION 453\r\nfactfinding was necessary.3 Here, as so often";
    const HLR_OPENING: &str = "1058 \r\nVOLUME 137 FEBRUARY 2024 NUMBER 4 \r\n© 2024 by The Harvard Law Review Association \r\nARTICLE \r\nCONTRACT-WRAPPED PROPERTY \r\nDanielle D’Onfro\r\nCONTENTS \r\nINTRODUCTION ....... 1059";
    const HLR_EVEN: &str =
        "1060 HARVARD LAW REVIEW [Vol. 137:1058\r\nno apps or subscriptions required.";
    const HLR_ODD: &str = "2024] CONTRACT-WRAPPED PROPERTY 1061\r\ntext";

    #[test]
    fn reads_an_nyu_opening_and_running_heads() {
        let identity = masthead_identity(&[NYU_OPENING, NYU_EVEN, NYU_ODD], None, &[], "file");
        assert_eq!(identity.title, "GENERATIVE INTERPRETATION");
        assert_eq!(identity.authors, "Yonathan Arbel & David A. Hoffman");
        assert_eq!(
            identity.citation,
            "99 New York University Law Review 451 · 2024"
        );
    }

    #[test]
    fn reads_a_harvard_opening_with_a_single_unmarked_author() {
        let identity = masthead_identity(&[HLR_OPENING, HLR_EVEN, HLR_ODD], None, &[], "file");
        assert_eq!(identity.title, "CONTRACT-WRAPPED PROPERTY");
        assert_eq!(identity.authors, "Danielle D’Onfro");
        assert_eq!(identity.citation, "137 Harvard Law Review 1058 · 2024");
    }

    #[test]
    fn the_opening_page_alone_still_yields_journal_volume_and_year() {
        let identity = masthead_identity(&[NYU_OPENING], None, &[], "file");
        assert_eq!(
            identity.citation,
            "99 New York University Law Review 451 · 2024"
        );
    }

    #[test]
    fn review_title_fills_in_when_the_page_text_is_not_loaded_and_loses_its_callouts() {
        let title = format!("Generative Interpretation{CALLOUT_START}1{CALLOUT_END}*");
        let identity = masthead_identity(&[], Some(&title), &[], "file");
        assert_eq!(identity.title, "Generative Interpretation");
    }

    #[test]
    fn a_review_title_that_is_really_the_institution_is_ignored() {
        let identity = masthead_identity(&[NYU_EVEN], Some("NEW YORK UNIVERSITY"), &[], "file");
        assert_eq!(identity.title, "file");
        let identity = masthead_identity(
            &["", NYU_EVEN, NYU_ODD],
            Some("NEW YORK UNIVERSITY LAW REVIEW"),
            &[],
            "file",
        );
        assert_eq!(identity.title, "GENERATIVE INTERPRETATION");
    }

    #[test]
    fn falls_back_to_the_file_name_for_ordinary_pdfs() {
        let identity = masthead_identity(
            &["Quarterly results\nRevenue grew in every region."],
            None,
            &[],
            "Q3 board memo",
        );
        assert_eq!(identity.title, "Q3 board memo");
        assert_eq!(identity.authors, "");
        assert_eq!(identity.citation, "");
    }

    #[test]
    fn review_blocks_supply_a_byline_when_the_page_text_has_none() {
        let blocks = vec![LiquidBlock {
            role: LiquidBlockRole::AuthorInfo,
            text: "SAMUEL D. WARREN* AND LOUIS D. BRANDEIS**".to_owned(),
            label: None,
        }];
        let identity = masthead_identity(&[], Some("The Right to Privacy"), &blocks, "file");
        assert_eq!(identity.authors, "SAMUEL D. WARREN & LOUIS D. BRANDEIS");
    }

    #[test]
    fn biography_lines_are_not_bylines() {
        assert!(!looks_like_byline(
            "† Professor of Law, University of Alabama School of Law."
        ));
        assert!(!looks_like_byline("Copyright © 2024 by Yonathan Arbel"));
        assert!(looks_like_byline("Yonathan Arbel† & David A. Hoffman‡"));
        assert!(!looks_like_byline("Danielle D’Onfro"));
        assert!(looks_like_byline("Danielle D’Onfro*"));
    }

    #[test]
    fn body_lines_with_brackets_are_not_running_heads() {
        assert_eq!(dated_title_head("See id. (quoting Smith, 2019] at 4"), None);
        assert_eq!(
            dated_title_head("May 2024] GENERATIVE INTERPRETATION 453"),
            Some((2024, "GENERATIVE INTERPRETATION".to_owned()))
        );
        assert_eq!(
            journal_volume_head("the court in [Vol. 3:12 of the reporter"),
            None
        );
    }

    #[test]
    fn non_ascii_running_heads_do_not_break_slicing() {
        assert_eq!(
            journal_volume_head("ÜBER STRASSE LAW REVIEW [Vol. 9:1"),
            Some(("ÜBER STRASSE LAW REVIEW".to_owned(), 9, 1))
        );
    }

    #[test]
    fn citation_degrades_gracefully() {
        assert_eq!(format_citation(None, None, None, None), "");
        assert_eq!(format_citation(None, None, None, Some(1890)), "1890");
        assert_eq!(
            format_citation(Some("HARVARD LAW REVIEW"), Some(4), Some(193), Some(1890)),
            "4 Harvard Law Review 193 · 1890"
        );
        assert_eq!(
            format_citation(Some("YALE LAW JOURNAL"), None, None, None),
            "Yale Law Journal"
        );
        assert_eq!(
            format_citation(Some("JOURNAL OF LAW AND ECONOMICS"), Some(12), None, None),
            "12 Journal of Law and Economics"
        );
    }
}
