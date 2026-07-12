//! Document-level profile classification for Liquid Mode.
//!
//! The classifier is intentionally local and explainable. It is a v1 scoring
//! model that can later be replaced by exported trained weights while keeping
//! the same public `DocumentProfile` surface.

use std::collections::BTreeMap;

use crate::liquid::model::{
    DocumentProfile, DocumentProfileKind, DocumentProfileScore, LiquidBlock, LiquidBlockRole,
};
use crate::liquid::util::word_count;

#[derive(Debug, Clone)]
pub(super) struct DocumentProfileInput<'a> {
    pub title: &'a str,
    pub source_text: &'a str,
    pub blocks: &'a [LiquidBlock],
    pub page_count: usize,
    pub extracted_pages: usize,
}

pub(super) fn classify_document_profile(input: DocumentProfileInput<'_>) -> DocumentProfile {
    if input.page_count > 0 && input.extracted_pages == 0 {
        return DocumentProfile {
            kind: DocumentProfileKind::ScannedImageOnly,
            confidence: 0.96,
            scores: vec![DocumentProfileScore {
                kind: DocumentProfileKind::ScannedImageOnly,
                score: 10.0,
            }],
            evidence: vec!["no selectable text on any page".to_owned()],
        };
    }
    let source_word_count = word_count(input.source_text);
    if input.page_count >= 4 && source_word_count < input.page_count * 15 {
        return DocumentProfile {
            kind: DocumentProfileKind::ScannedImageOnly,
            confidence: 0.9,
            scores: vec![DocumentProfileScore {
                kind: DocumentProfileKind::ScannedImageOnly,
                score: 8.5,
            }],
            evidence: vec!["near-zero selectable words per page".to_owned()],
        };
    }

    let features = ProfileFeatures::from_input(input);
    let mut scored = ProfileScores::default();

    score_law_review(&features, &mut scored);
    score_science_article(&features, &mut scored);
    score_contract(&features, &mut scored);
    score_legal_filing_or_opinion(&features, &mut scored);
    score_news_article(&features, &mut scored);
    score_free_prose(&features, &mut scored);
    score_cv_or_academic_packet(&features, &mut scored);
    score_receipt_invoice_financial(&features, &mut scored);
    score_course_or_exam_material(&features, &mut scored);
    score_book_or_chapter(&features, &mut scored);
    score_policy_report(&features, &mut scored);
    score_form_receipt_admin(&features, &mut scored);
    score_general_document(&features, &mut scored);

    scored.finish()
}

#[derive(Debug)]
struct ProfileFeatures {
    title_lower: String,
    text_lower: String,
    block_text_lower: String,
    word_count: usize,
    page_count: usize,
    extracted_pages: usize,
    role_counts: BTreeMap<LiquidBlockRole, usize>,
    block_count: usize,
}

impl ProfileFeatures {
    fn from_input(input: DocumentProfileInput<'_>) -> Self {
        let mut role_counts = BTreeMap::new();
        for block in input.blocks {
            *role_counts.entry(block.role).or_insert(0) += 1;
        }
        let block_text_lower = input
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();

        Self {
            title_lower: input.title.to_ascii_lowercase(),
            text_lower: input.source_text.to_ascii_lowercase(),
            block_text_lower,
            word_count: word_count(input.source_text),
            page_count: input.page_count,
            extracted_pages: input.extracted_pages,
            role_counts,
            block_count: input.blocks.len(),
        }
    }

    fn contains_any(&self, needles: &[&str]) -> bool {
        needles.iter().any(|needle| {
            self.text_lower.contains(needle)
                || self.title_lower.contains(needle)
                || self.block_text_lower.contains(needle)
        })
    }

    fn starts_title_any(&self, needles: &[&str]) -> bool {
        needles
            .iter()
            .any(|needle| self.title_lower.starts_with(needle))
    }

    fn title_contains_any(&self, needles: &[&str]) -> bool {
        needles
            .iter()
            .any(|needle| self.title_lower.contains(needle))
    }

    fn count_any(&self, needles: &[&str]) -> usize {
        needles
            .iter()
            .map(|needle| self.text_lower.matches(needle).count())
            .sum()
    }

    fn role_count(&self, role: LiquidBlockRole) -> usize {
        self.role_counts.get(&role).copied().unwrap_or_default()
    }

    fn role_ratio(&self, role: LiquidBlockRole) -> f32 {
        if self.block_count == 0 {
            return 0.0;
        }
        self.role_count(role) as f32 / self.block_count as f32
    }
}

#[derive(Default)]
struct ProfileScores {
    values: BTreeMap<DocumentProfileKind, f32>,
    evidence: BTreeMap<DocumentProfileKind, Vec<String>>,
}

impl ProfileScores {
    fn add(&mut self, kind: DocumentProfileKind, amount: f32, evidence: impl Into<String>) {
        *self.values.entry(kind).or_insert(0.0) += amount;
        self.evidence.entry(kind).or_default().push(evidence.into());
    }

    fn finish(self) -> DocumentProfile {
        let mut scores = self
            .values
            .into_iter()
            .map(|(kind, score)| DocumentProfileScore {
                kind,
                score: round_score(score),
            })
            .collect::<Vec<_>>();
        scores.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.kind.cmp(&right.kind))
        });

        let top = scores.first().cloned().unwrap_or(DocumentProfileScore {
            kind: DocumentProfileKind::Other,
            score: 0.0,
        });
        let second = scores.get(1).map(|score| score.score).unwrap_or_default();
        let total = scores.iter().map(|score| score.score.max(0.0)).sum::<f32>();
        let mut kind = top.kind;
        if top.score < 2.0 {
            kind = DocumentProfileKind::Other;
        }

        let raw_confidence = if total <= f32::EPSILON {
            0.25
        } else {
            let margin = (top.score - second).max(0.0);
            0.35 + (top.score / total) * 0.45 + (margin / (top.score + 1.0)) * 0.20
        };
        let confidence = raw_confidence.clamp(0.25, 0.96);
        let evidence = self
            .evidence
            .get(&top.kind)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(6)
            .collect::<Vec<_>>();

        DocumentProfile {
            kind,
            confidence: round_score(confidence),
            scores,
            evidence,
        }
    }
}

fn score_law_review(features: &ProfileFeatures, scores: &mut ProfileScores) {
    let short_law_review_forum_piece = looks_like_short_law_review_forum_piece(features);
    if features.starts_title_any(&["letter to ", "reply to ", "response to "])
        && features.page_count <= 8
        && !short_law_review_forum_piece
    {
        return;
    }
    let large_law_review_apparatus = has_large_law_review_note_apparatus(features);
    if !large_law_review_apparatus
        && !short_law_review_forum_piece
        && (looks_like_legal_research_results(features)
            || looks_like_formal_letter_body(features)
            || looks_like_ali_restatement(features))
    {
        return;
    }
    if features.title_contains_any(&[
        "curriculum vitae",
        "resume",
        "letter of support",
        "reference letter",
        "recommendation letter",
        "cover letter",
        "research agenda",
        "tenure report",
        "annual review",
        "publication agreement",
    ]) {
        return;
    }
    if looks_like_strong_academic_packet_body(features) && !large_law_review_apparatus {
        return;
    }

    if looks_like_law_review_repository_article(features) {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            3.2,
            "law-review repository article marker",
        );
    }
    if short_law_review_forum_piece {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            4.8,
            "law-review forum piece with note apparatus",
        );
    }
    if (!features.starts_title_any(&["letter to ", "reply to ", "response to "])
        || short_law_review_forum_piece)
        && features.contains_any(&[
            "law review",
            "law journal",
            "l. rev",
            "l. j.",
            " l j ",
            "harv. l. rev",
            "yale l. j",
            "yale law journal",
            "harvard law review",
            "ssrn.com/abstract",
        ])
    {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            2.4,
            "law-review or SSRN marker",
        );
    }
    let note_count = law_review_note_count(features);
    if note_count >= 8 {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            1.8,
            "high note density",
        );
    }
    if note_count >= 14 && features.role_count(LiquidBlockRole::Paragraph) >= 18 {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            1.2,
            "dense footnoted legal article structure",
        );
    }
    if large_law_review_apparatus {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            2.6,
            "large law-review note apparatus",
        );
    }
    if features.title_lower.starts_with("article ") && note_count >= 40 {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            1.4,
            "law-review article heading plus notes",
        );
    }
    if features.contains_any(&[
        "abstract",
        "introduction",
        "citation",
        "jstor",
        "heinonline",
    ]) {
        scores.add(
            DocumentProfileKind::LawReviewArticle,
            0.9,
            "academic legal front matter",
        );
    }
}

fn score_science_article(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_legal_research_results(features)
        || looks_like_law_review_repository_article(features)
        || looks_like_ali_restatement(features)
    {
        return;
    }
    if looks_like_researchgate_article(features) {
        scores.add(
            DocumentProfileKind::ScienceArticle,
            2.8,
            "ResearchGate or academic article front matter",
        );
    }
    if features.contains_any(&[
        "doi:",
        "https://doi.org/",
        "keywords",
        "received:",
        "accepted:",
        "published online",
    ]) {
        scores.add(
            DocumentProfileKind::ScienceArticle,
            2.0,
            "DOI, keywords, or publication-history marker",
        );
    }
    if features.contains_any(&[
        "methods",
        "materials and methods",
        "results",
        "discussion",
        "references",
        "springer",
        "elsevier",
    ]) {
        scores.add(
            DocumentProfileKind::ScienceArticle,
            1.4,
            "science article section or publisher marker",
        );
    }
    if features.role_count(LiquidBlockRole::Abstract) > 0
        && features.role_count(LiquidBlockRole::Metadata) >= 2
    {
        scores.add(
            DocumentProfileKind::ScienceArticle,
            1.0,
            "abstract plus metadata",
        );
    }
}

fn score_contract(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_researchgate_article(features)
        || looks_like_dense_law_review_article(features)
        || looks_like_law_review_repository_article(features)
        || looks_like_formal_letter_body(features)
        || looks_like_expense_report(features)
        || looks_like_travel_or_receipt_packet(features)
        || looks_like_book_or_manual_material(features)
        || looks_like_uscis_notice(features)
        || looks_like_state_business_filing(features)
        || looks_like_ali_restatement(features)
    {
        return;
    }
    if features.contains_any(&[
        "tax return",
        "form 1040",
        "adjusted gross income",
        "taxable income",
        "state income tax",
        "alabama individual income tax",
        "federal return",
    ]) {
        return;
    }

    if features.title_contains_any(&[
        "master services agreement",
        "services agreement",
        "speaker-panelist agreement",
        "publication agreement",
        "confidentiality agreement",
        "lease extension",
        "addendum to offer letter",
        "statement of work",
    ]) || features.contains_any(&[
        "whereas,",
        "in witness whereof",
        "master services agreement",
        "this agreement is made",
        "agreement between",
        "party shall",
        "parties agree",
    ]) {
        scores.add(
            DocumentProfileKind::Contract,
            2.4,
            "contract formation marker",
        );
    }
    if features.title_contains_any(&["agreement", "lease", "contract"])
        && features.contains_any(&["effective date", "signature", "governing law", "party"])
    {
        scores.add(
            DocumentProfileKind::Contract,
            1.7,
            "contract title plus execution/party marker",
        );
    }
    if features.role_count(LiquidBlockRole::KeyClause) >= 2
        || features.role_count(LiquidBlockRole::Clause) >= 2
        || features.role_count(LiquidBlockRole::Marginalia) >= 3
    {
        scores.add(
            DocumentProfileKind::Contract,
            1.8,
            "contract-style clauses or field rows",
        );
    }
    if features.contains_any(&["exhibit ", "schedule ", "signature", "effective date"]) {
        scores.add(
            DocumentProfileKind::Contract,
            0.8,
            "contract attachment/signature marker",
        );
    }
}

fn score_legal_filing_or_opinion(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_ali_restatement(features) || looks_like_book_or_manual_material(features) {
        return;
    }
    if features.title_contains_any(&[
        "curriculum vitae",
        "resume",
        "academic resume",
        "tenure report",
        "annual review",
        "promotion report",
    ]) {
        return;
    }
    if features.contains_any(&[
        "academic appointments",
        "selected publications",
        "courses taught",
        "bar admissions",
        "professional service",
        "professional strengths",
        "teaching experience",
        "teaching, research, and service interests",
        "curriculum vitae",
        "academic position",
        "academic positions",
    ]) {
        return;
    }
    if looks_like_course_evaluation_body(features)
        || looks_like_dense_law_review_article(features)
        || looks_like_legal_research_results(features)
        || looks_like_law_review_repository_article(features)
        || looks_like_researchgate_article(features)
        || looks_like_chat_export(features)
        || looks_like_codebook_or_data_dictionary(features)
    {
        return;
    }

    if features.contains_any(&[
        "united states district court",
        "supreme court",
        "court of appeals",
        "plaintiff",
        "defendant",
        "petitioner",
        "respondent",
        "memorandum opinion",
        "order granting",
        "case no.",
        "syllabus",
    ]) {
        scores.add(
            DocumentProfileKind::LegalFilingOrOpinion,
            2.3,
            "court, party, or case-caption marker",
        );
    }
    if features.contains_any(&[" v. ", "motion to ", "brief of ", "judgment", "docket"]) {
        scores.add(
            DocumentProfileKind::LegalFilingOrOpinion,
            1.3,
            "litigation citation or filing marker",
        );
    }
}

fn score_news_article(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_formal_letter_body(features)
        || looks_like_property_deal_sheet(features)
        || looks_like_legal_research_results(features)
        || looks_like_law_review_repository_article(features)
        || looks_like_ali_restatement(features)
    {
        return;
    }

    if features.contains_any(&[
        "published ",
        "updated ",
        "last updated",
        "reuters",
        "associated press",
        "photo:",
        "source:",
        "the new york times",
        "washington post",
    ]) {
        scores.add(
            DocumentProfileKind::NewsArticle,
            1.9,
            "news date/source/photo marker",
        );
    }
    if looks_like_blog_page(features) {
        scores.add(
            DocumentProfileKind::NewsArticle,
            2.6,
            "blog or web article marker",
        );
    }
    if features.role_count(LiquidBlockRole::Caption) > 0
        || features.role_count(LiquidBlockRole::Lead) > 0
    {
        scores.add(
            DocumentProfileKind::NewsArticle,
            0.8,
            "caption or lead block",
        );
    }
}

fn score_free_prose(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_ali_restatement(features) {
        return;
    }
    if looks_like_formal_letter_body(features) {
        scores.add(
            DocumentProfileKind::FreeProse,
            3.4,
            "formal letter body marker",
        );
    }
    if features.title_contains_any(&["teaching philosophy"]) {
        scores.add(
            DocumentProfileKind::FreeProse,
            3.8,
            "teaching philosophy title marker",
        );
    }
    if features.starts_title_any(&["letter to ", "reply to ", "response to "]) {
        scores.add(
            DocumentProfileKind::FreeProse,
            3.2,
            "letter or response title",
        );
    }
    if features.starts_title_any(&["essay"])
        || features.contains_any(&["dear ", "sincerely,", "i never ", "i am writing"])
    {
        scores.add(
            DocumentProfileKind::FreeProse,
            2.0,
            "letter, essay, or first-person prose marker",
        );
    }
    if features.role_ratio(LiquidBlockRole::Paragraph) > 0.45
        && features.role_count(LiquidBlockRole::Heading) <= 3
        && features.word_count > 250
    {
        scores.add(
            DocumentProfileKind::FreeProse,
            1.1,
            "paragraph-heavy document with few headings",
        );
    }
}

fn score_cv_or_academic_packet(features: &ProfileFeatures, scores: &mut ProfileScores) {
    let strong_academic_packet = looks_like_strong_academic_packet_body(features)
        || looks_like_faculty_application_packet(features);
    if looks_like_state_business_filing(features) {
        return;
    }
    if looks_like_researchgate_article(features)
        || looks_like_legal_research_results(features)
        || has_large_law_review_note_apparatus(features)
        || (looks_like_law_review_repository_article(features) && !strong_academic_packet)
    {
        return;
    }
    if features.title_contains_any(&[
        "curriculum vitae",
        "resume",
        "academic resume",
        "academic position",
        "academic positions",
        "application:",
        "faculty application",
        "far form",
        "cv",
        "letter of support",
        "reference letter",
        "recommendation letter",
        "cover letter",
        "research agenda",
        "teaching statement",
        "diversity statement",
        "tenure report",
        "promotion report",
        "annual review",
    ]) {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            4.4,
            "CV, appointment, or academic-packet title marker",
        );
    }
    if features.contains_any(&[
        "curriculum vitae",
        "education",
        "publications",
        "selected publications",
        "academic appointments",
        "academic c.v.",
        "academic cv",
        "academic position",
        "academic positions",
        "faculty application",
        "works in progress",
        "legal and practice experience",
        "present position & prior academic employment",
        "prior academic employment",
        "resume / curriculum",
        "teaching evaluations",
        "teaching experience",
        "teaching, research, and service interests",
        "research agenda",
        "teaching statement",
        "courses taught",
        "bar admissions",
        "professional service",
        "professional strengths",
    ]) {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            2.4,
            "academic packet body marker",
        );
    }
    if looks_like_faculty_application_packet(features) {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            4.2,
            "faculty application packet marker",
        );
    }
    if looks_like_strong_academic_packet_body(features) {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            4.0,
            "strong CV or academic-packet body structure",
        );
    }
    if title_has_academic_credentials(features) && strong_academic_packet {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            1.4,
            "credentialed academic name with CV body",
        );
    }
    if looks_like_list_heavy_credentialed_cv(features) {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            2.0,
            "list-heavy credentialed CV structure",
        );
    }
    if features.role_ratio(LiquidBlockRole::Heading) > 0.35
        && count_academic_packet_markers(features) >= 3
    {
        scores.add(
            DocumentProfileKind::CvOrAcademicPacket,
            1.8,
            "heading-heavy academic packet structure",
        );
    }
}

fn score_receipt_invoice_financial(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_researchgate_article(features)
        || looks_like_dense_law_review_article(features)
        || looks_like_uscis_notice(features)
    {
        return;
    }
    let event_logistics_packet = looks_like_event_logistics_packet(features);
    if looks_like_expense_report(features) {
        scores.add(
            DocumentProfileKind::ReceiptInvoiceFinancial,
            4.8,
            "expense report body marker",
        );
    }
    if looks_like_travel_or_receipt_packet(features) {
        scores.add(
            DocumentProfileKind::ReceiptInvoiceFinancial,
            4.6,
            "travel receipt or itinerary marker",
        );
    }
    if features.title_contains_any(&[
        "receipt",
        "reciept",
        "reciepts",
        "invoice",
        "expense",
        "bill",
        "pay stub",
        "paystub",
        "federal return",
        "federalworksheets",
        "tax return",
        "ledger",
        "estimate",
        "quote",
    ]) {
        scores.add(
            DocumentProfileKind::ReceiptInvoiceFinancial,
            4.2,
            "receipt, invoice, tax, or financial title marker",
        );
    }
    if !event_logistics_packet
        && features.page_count <= 12
        && features.contains_any(&[
            "receipt",
            "invoice",
            "payment method",
            "subtotal",
            "total $",
            "transaction",
            "amount due",
            "tax return",
        ])
    {
        scores.add(
            DocumentProfileKind::ReceiptInvoiceFinancial,
            2.4,
            "short receipt/invoice/payment body marker",
        );
    }
    if features.contains_any(&[
        "tax return",
        "form 1040",
        "adjusted gross income",
        "taxable income",
        "state income tax",
        "alabama individual income tax",
        "federal return",
        "withholding",
        "refund due",
    ]) {
        scores.add(
            DocumentProfileKind::ReceiptInvoiceFinancial,
            3.5,
            "tax return or financial filing marker",
        );
    }
}

fn score_course_or_exam_material(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_dense_law_review_article(features) {
        return;
    }
    if features.title_contains_any(&[
        "syllabus",
        "exam",
        "midterm",
        "final exam",
        "assignment",
        "course",
        "eval",
        "evals",
        "evaluation",
        "evaluations",
        "law-",
        "contracts assignment",
    ]) {
        scores.add(
            DocumentProfileKind::CourseOrExamMaterial,
            3.6,
            "course, exam, syllabus, or assignment title marker",
        );
    }
    if features.contains_any(&[
        "class meeting",
        "course schedule",
        "office hours",
        "final exam",
        "midterm exam",
        "assignment due",
        "anonymous number",
        "course evaluation",
        "student evaluation",
        "survey comparisons",
        "responses / expected",
        "responsible faculty",
        "overall mean",
    ]) {
        scores.add(
            DocumentProfileKind::CourseOrExamMaterial,
            2.6,
            "course/exam body marker",
        );
    }
    if looks_like_course_evaluation_body(features) {
        scores.add(
            DocumentProfileKind::CourseOrExamMaterial,
            4.0,
            "strong course-evaluation body marker",
        );
    }
    if looks_like_syllabus_body(features) {
        scores.add(
            DocumentProfileKind::CourseOrExamMaterial,
            4.0,
            "syllabus/course schedule body marker",
        );
    }
}

fn score_book_or_chapter(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_researchgate_article(features)
        || looks_like_legal_research_results(features)
        || looks_like_dense_law_review_article(features)
        || looks_like_law_review_repository_article(features)
    {
        return;
    }
    if looks_like_ali_restatement(features) || features.title_contains_any(&["restatement"]) {
        scores.add(
            DocumentProfileKind::BookOrChapter,
            5.0,
            "ALI Restatement or treatise title marker",
        );
    }
    if features.contains_any(&[
        "restatement (second)",
        "restatement (third)",
        "restatement of",
        "american law institute",
    ]) {
        scores.add(
            DocumentProfileKind::BookOrChapter,
            3.2,
            "treatise/restatement marker",
        );
    }
    if features.title_contains_any(&["isbn", "book", "chapter"]) {
        scores.add(
            DocumentProfileKind::BookOrChapter,
            2.8,
            "book or chapter title marker",
        );
    }
    if looks_like_book_or_manual_material(features) {
        scores.add(
            DocumentProfileKind::BookOrChapter,
            3.6,
            "book chapter, manual, or guide marker",
        );
    }
    if features.contains_any(&[
        "isbn",
        "university press",
        "oxford university press",
        "cambridge university press",
        "routledge",
        "penguin books",
    ]) {
        scores.add(
            DocumentProfileKind::BookOrChapter,
            2.0,
            "book publisher or ISBN marker",
        );
    }
}

fn score_policy_report(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_law_review_repository_article(features) {
        return;
    }
    if features.title_contains_any(&[
        "policy report",
        "research report",
        "white paper",
        "economic survey",
        "gao",
        "oecd",
    ]) {
        scores.add(
            DocumentProfileKind::PolicyReport,
            3.2,
            "policy/research report title marker",
        );
    }
    if features.contains_any(&[
        "report to congressional",
        "gao",
        "oecd economic surveys",
        "policy report",
        "white paper",
        "working paper",
        "executive summary",
    ]) {
        scores.add(
            DocumentProfileKind::PolicyReport,
            1.9,
            "policy report body marker",
        );
    }
}

fn score_form_receipt_admin(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if features.title_contains_any(&[
        "curriculum vitae",
        "resume",
        "academic resume",
        "far form",
        "cover sheet",
        "schedule",
        "syllabus",
        "annual review",
        "email",
        "gmail",
    ]) {
        scores.add(
            DocumentProfileKind::FormReceiptAdmin,
            4.2,
            "administrative title marker",
        );
    }
    if features.word_count < 650
        && features.count_any(&["date", "name", "email", "address", "phone"]) >= 3
    {
        scores.add(
            DocumentProfileKind::FormReceiptAdmin,
            1.0,
            "short administrative form document",
        );
    }
    if looks_like_uscis_notice(features) {
        scores.add(
            DocumentProfileKind::FormReceiptAdmin,
            5.0,
            "USCIS notice or request-for-evidence marker",
        );
    }
    if looks_like_state_business_filing(features) {
        scores.add(
            DocumentProfileKind::FormReceiptAdmin,
            5.0,
            "state business filing form marker",
        );
    }
}

fn score_general_document(features: &ProfileFeatures, scores: &mut ProfileScores) {
    if looks_like_ali_restatement(features) {
        return;
    }
    if looks_like_event_logistics_packet(features) {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            6.0,
            "event logistics packet marker",
        );
    }
    if looks_like_chat_export(features) {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            3.4,
            "chat or AI-answer export marker",
        );
    }
    if looks_like_codebook_or_data_dictionary(features) {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            3.8,
            "codebook or data dictionary marker",
        );
    }
    if looks_like_property_deal_sheet(features) {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            3.4,
            "property deal sheet marker",
        );
    }
    if looks_like_legal_research_results(features) {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            3.5,
            "legal research result-list marker",
        );
    }
    if features.extracted_pages > 0 {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            1.0,
            "selectable text exists",
        );
    }
    if features.page_count <= 2 && features.word_count < 400 {
        scores.add(
            DocumentProfileKind::GeneralDocument,
            0.4,
            "short general document",
        );
    }
}

fn looks_like_strong_academic_packet_body(features: &ProfileFeatures) -> bool {
    let marker_count = count_academic_packet_markers(features);
    marker_count >= 4
        || features.contains_any(&[
            "academic appointments",
            "academic c.v.",
            "academic cv",
            "academic position",
            "academic positions",
            "current appointment",
            "professional appointments",
        ]) && features.contains_any(&["publications", "selected publications", "education"])
        || features.contains_any(&["courses taught", "teaching:"])
            && features.contains_any(&["professional service", "bar admissions", "publications"])
        || features.contains_any(&["professional strengths"])
            && features.contains_any(&["teaching experience", "education"])
            && features.contains_any(&["journal articles", "book chapters", "publications"])
}

fn title_has_academic_credentials(features: &ProfileFeatures) -> bool {
    [" jd", " j.d.", " llm", " ll.m.", " phd", " ph.d."]
        .iter()
        .any(|credential| features.title_lower.contains(credential))
}

fn looks_like_list_heavy_credentialed_cv(features: &ProfileFeatures) -> bool {
    title_has_academic_credentials(features)
        && count_academic_packet_markers(features) >= 4
        && (features.role_count(LiquidBlockRole::ListItem) >= 20
            || features.role_ratio(LiquidBlockRole::ListItem) >= 0.45)
}

fn looks_like_faculty_application_packet(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "faculty application",
        "form: faculty application",
        "application:",
    ]) && features.contains_any(&[
        "required documents",
        "resume / curriculum",
        "cover letter",
        "teaching philosophy",
        "present position & prior academic employment",
        "academic c.v.",
        "academic cv",
    ])
}

fn count_academic_packet_markers(features: &ProfileFeatures) -> usize {
    [
        "curriculum vitae",
        "education",
        "publications",
        "selected publications",
        "academic appointments",
        "academic c.v.",
        "academic cv",
        "academic position",
        "academic positions",
        "current appointment",
        "professional appointments",
        "faculty application",
        "present position & prior academic employment",
        "prior academic employment",
        "resume / curriculum",
        "courses taught",
        "teaching:",
        "teaching experience",
        "teaching, research, and service interests",
        "bar admissions",
        "professional service",
        "professional strengths",
        "works in progress",
        "legal and practice experience",
        "research agenda",
        "teaching statement",
    ]
    .iter()
    .filter(|marker| {
        features.text_lower.contains(**marker) || features.title_lower.contains(**marker)
    })
    .count()
}

fn looks_like_course_evaluation_body(features: &ProfileFeatures) -> bool {
    features.count_any(&[
        "strongly agree",
        "strongly disagree",
        "neither agree nor disagree",
        "response rate mean",
        "school mean",
        "learning outcomes",
        "course content",
        "survey comparisons",
        "responses / expected",
        "overall mean",
    ]) >= 2
        || features.contains_any(&["response rate mean"])
            && features.contains_any(&["syllabus", "course", "mean"])
}

fn looks_like_syllabus_body(features: &ProfileFeatures) -> bool {
    features.contains_any(&["syllabus"])
        && features.contains_any(&[
            "course materials",
            "problem set",
            "casebook",
            "assignments",
            "attendance and preparation",
        ])
}

fn looks_like_legal_research_results(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "westlaw advantage",
        "westlaw edge",
        "keycite",
        "thomson reuters",
        "for educational use only",
    ]) && features.contains_any(&["list of ", "results for", "search result", "result list"])
        || features.contains_any(&["list of "])
            && features.contains_any(&["results for adv", "citing reference"])
            && features.contains_any(&["no claim to original", "thomson reuters"])
}

fn looks_like_dense_law_review_article(features: &ProfileFeatures) -> bool {
    has_large_law_review_note_apparatus(features)
}

fn has_large_law_review_note_apparatus(features: &ProfileFeatures) -> bool {
    let note_count = law_review_note_count(features);
    note_count >= 40
        && features.role_count(LiquidBlockRole::Paragraph) >= 40
        && (has_law_review_marker(features)
            || features.title_lower.starts_with("article ")
            || features.contains_any(&["article contents", "introduction", "abstract"]))
        && !features.title_contains_any(&[
            "curriculum vitae",
            "resume",
            "academic resume",
            "faculty application",
            "tenure report",
            "annual review",
            "promotion report",
        ])
}

fn law_review_note_count(features: &ProfileFeatures) -> usize {
    features.role_count(LiquidBlockRole::Footnote)
        + features.role_count(LiquidBlockRole::Marginalia)
}

fn has_law_review_marker(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "law review",
        "law journal",
        "l. rev",
        "l. j.",
        " l j ",
        "harv. l. rev",
        "yale l. j",
        "ssrn.com/abstract",
        "heinonline",
    ])
}

fn looks_like_short_law_review_forum_piece(features: &ProfileFeatures) -> bool {
    features.starts_title_any(&["letter to ", "reply to ", "response to "])
        && features.page_count <= 8
        && has_law_review_marker(features)
        && law_review_note_count(features) >= 8
        && features.role_count(LiquidBlockRole::Paragraph) >= 6
        && !features.title_contains_any(&[
            "letter of support",
            "reference letter",
            "recommendation letter",
            "cover letter",
        ])
}

fn looks_like_law_review_repository_article(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "institutional repository",
        "recommended citation",
        "law review",
    ]) && features.contains_any(&[
        "available at:",
        "part of the",
        "j.d. candidate",
        "introduction",
    ]) || features.contains_any(&["law review"])
        && features.contains_any(&[
            "recommended citation",
            "brought to you for free and open access",
            "accepted for inclusion",
        ])
}

fn looks_like_ali_restatement(features: &ProfileFeatures) -> bool {
    features.title_contains_any(&["restatement"])
        || features.contains_any(&["restatement of the law"])
        || features.contains_any(&["american law institute"])
            && features.contains_any(&["tentative draft", "subjects covered", "annual meeting"])
}

fn looks_like_expense_report(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "expense report",
        "project expense report",
        "expense register",
        "receipt evidence",
        "spend by category",
    ]) && features.contains_any(&["vendor", "category", "amount"])
}

fn looks_like_travel_or_receipt_packet(features: &ProfileFeatures) -> bool {
    features.title_contains_any(&["receipt", "reciept", "reciepts", "receipts pics"])
        || features.contains_any(&[
            "departing flight information",
            "american airlines",
            "hartsfield-jackson",
            "airport (",
            "flight ",
            "aircraft",
            "boarding",
            "itinerary",
            "confirmation number",
        ]) && features
            .contains_any(&["payment", "total", "amount", "hotel", "airport", "flight"])
}

fn looks_like_book_or_manual_material(features: &ProfileFeatures) -> bool {
    features.title_contains_any(&[
        "guide to ",
        "manual",
        "handbook",
        "palgrave",
        "causation in tort law",
    ]) || features.contains_any(&[
        "palgrave",
        "pearson education limited",
        "longman-elt",
        "preface to the first edition",
        "this book has been designed",
        "general rules for installation",
        "rules for underground installations",
        "code ref.",
        "minimum burial depth",
    ]) || features.contains_any(&["chapter", "references", "bibliography"])
        && features.contains_any(&["university press", "palgrave", "edited by"])
}

fn looks_like_uscis_notice(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "u.s. citizenship and immigration services",
        "us. citizenship",
        "immigration services",
        "department of homeland security",
        "request for evidence",
        "petition for a nonimmigrant worker",
        "unique receipt number",
        "immigrant investor program office",
    ]) && features.contains_any(&[
        "request for evidence",
        "petition for a nonimmigrant worker",
        "unique receipt number",
        "immigrant investor program office",
    ])
}

fn looks_like_state_business_filing(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "state of alabama",
        "domestic nonprofit corporation",
        "certificate of amendment",
        "secretary of state",
        "business services",
        "entity id number",
        "certificate of incorporation",
    ]) && features.contains_any(&[
        "secretary of state",
        "certificate of amendment",
        "entity id number",
        "nonprofit corporation",
    ])
}

fn looks_like_blog_page(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "prawfsblawg.blogs.com",
        "blogs.com/",
        "posted by:",
        "comments",
        "permalink",
    ]) && features.contains_any(&["http://", "https://", "blog", "posted by:"])
}

fn looks_like_formal_letter_body(features: &ProfileFeatures) -> bool {
    features.contains_any(&["dear "])
        && features.contains_any(&[
            "i am writing",
            "this letter",
            "professional opinion",
            "u.s. citizenship and immigration services",
            "mailstop",
            "sincerely",
        ])
}

fn looks_like_researchgate_article(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "researchgate.net/publication",
        "see discussions, stats, and author profiles",
    ]) || features.contains_any(&["article ·", "citations", "reads"])
        && features.contains_any(&[
            "see profile",
            "all content following this page was uploaded",
        ])
}

fn looks_like_chat_export(features: &ProfileFeatures) -> bool {
    features.contains_any(&["chatgpt.com/c/", "thought for "])
        && features.contains_any(&[
            "great question",
            "here's what",
            "here’s what",
            "how people actually make",
            "1 of ",
        ])
}

fn looks_like_codebook_or_data_dictionary(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        " codebook",
        "codebook ",
        "data dictionary",
        "list of variables",
        "variable names",
        "each variable to be coded",
        "measurement level:",
        "print format:",
        "write format:",
    ]) || features.contains_any(&["description:", "type:", "format:", "notes:"])
        && features.count_any(&["description:", "type:", "format:", "notes:"]) >= 6
}

fn looks_like_property_deal_sheet(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "off market deals",
        "cash or hard money",
        "seller financing",
        "inspection period",
    ]) && features.contains_any(&["price:", "beds:", "baths:", "sqft:", "year built:"])
        || features.count_any(&[
            "price:",
            "arv:",
            "beds:",
            "baths:",
            "sqft:",
            "year built:",
            "lot size:",
            "occupancy:",
        ]) >= 8
}

fn looks_like_event_logistics_packet(features: &ProfileFeatures) -> bool {
    features.contains_any(&[
        "roundtable",
        "conference",
        "symposium",
        "workshop",
        "seminar",
        "summit",
    ]) && features.contains_any(&[
        "dear participants",
        "logistics",
        "accommodation options",
        "booking deadline",
        "schedule highlights",
        "contact information for logistical issues",
    ])
}

fn round_score(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    fn classify(title: &str, text: &str, blocks: Vec<LiquidBlock>) -> DocumentProfile {
        classify_document_profile(DocumentProfileInput {
            title,
            source_text: text,
            blocks: &blocks,
            page_count: 3,
            extracted_pages: 3,
        })
    }

    #[test]
    fn classifies_contracts_from_clause_and_agreement_signals() {
        let profile = classify(
            "Master Services Agreement",
            "This Agreement is made between the parties. The customer shall pay the fees. IN WITNESS WHEREOF.",
            vec![
                block(LiquidBlockRole::Title, "Master Services Agreement"),
                block(
                    LiquidBlockRole::KeyClause,
                    "The customer shall pay the fees.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "Effective Date: January 1, 2026",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::Contract);
        assert!(profile.confidence >= 0.5);
    }

    #[test]
    fn classifies_law_review_articles_from_footnotes_and_journal_markers() {
        let mut blocks = vec![block(LiquidBlockRole::Title, "The Death of Liability")];
        for index in 1..=9 {
            blocks.push(block(
                LiquidBlockRole::Footnote,
                &format!("{index}. See Example Law Review 1 (2026)."),
            ));
        }
        let profile = classify(
            "The Death of Liability",
            "Source: The Yale Law Journal. This Article appears on SSRN.com/abstract=123. Introduction.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_dense_footnoted_legal_articles_confidently() {
        let mut blocks = vec![block(
            LiquidBlockRole::Title,
            "A Question of Intent: Aiding and Abetting Law",
        )];
        for index in 1..=20 {
            blocks.push(block(
                LiquidBlockRole::Footnote,
                &format!("{index}. See 18 U.S.C. § 924(c); Smith v. Jones."),
            ));
        }
        for _ in 0..24 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                "This article analyzes accomplice liability, statutory intent, and federal criminal doctrine.",
            ));
        }
        let profile = classify(
            "A Question of Intent: Aiding and Abetting Law",
            "Firearms are common tools of violent-crime and drug-trafficking trades. Courts review 18 U.S.C. § 924(c). Smith v. Jones.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_dense_marginalia_law_review_over_course_survey_markers() {
        let mut blocks = vec![block(LiquidBlockRole::Title, "Testing Ordinary Meaning")];
        for index in 1..=80 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{index} See 134 Harv. L. Rev. 726 (2020)."),
            ));
        }
        for _ in 0..55 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                "This article studies ordinary meaning, statutory interpretation, and survey responses.",
            ));
        }
        let profile = classify(
            "Testing Ordinary Meaning",
            "Kevin P. Tobia, Testing Ordinary Meaning, 134 HARV. L. REV. 726 (2020). Introduction. Strongly agree. Strongly disagree. Course content. Survey comparisons. Overall mean.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_article_heading_with_dense_marginalia_over_tax_language() {
        let mut blocks = vec![block(
            LiquidBlockRole::Title,
            "ARTICLE PRAGMATIC FAMILY LAW",
        )];
        for index in 1..=90 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{index} See 136 Harv. L. Rev. 1501 (2023)."),
            ));
        }
        for _ in 0..60 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                "The article develops family-law doctrine through institutional and pragmatic analysis.",
            ));
        }
        let profile = classify(
            "ARTICLE PRAGMATIC FAMILY LAW",
            "INTRODUCTION. This Article analyzes family law. Some examples discuss state income tax and federal return policy.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_dense_law_review_over_contract_language() {
        let mut blocks = vec![block(
            LiquidBlockRole::Title,
            "Chevron and the Canon Favoring Indians",
        )];
        for index in 1..=70 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{index} See 65 U. Chi. L. Rev. 1 (1998)."),
            ));
        }
        for _ in 0..50 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                "The article analyzes statutory interpretation and administrative-law canons.",
            ));
        }
        let profile = classify(
            "Chevron and the Canon Favoring Indians",
            "INTRODUCTION. Since 1832, courts have interpreted ambiguous statutes. Whereas parties agree in some private contracts, this Article concerns public law.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_dense_law_review_over_restatement_cues() {
        let mut blocks = vec![block(
            LiquidBlockRole::Title,
            "ARTICLE CONTRACT-WRAPPED PROPERTY",
        )];
        for index in 1..=85 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{index} See 137 Harv. L. Rev. 1058 (2024)."),
            ));
        }
        for _ in 0..65 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                "This article analyzes private law, ownership, and contract doctrines.",
            ));
        }
        let profile = classify(
            "ARTICLE CONTRACT-WRAPPED PROPERTY",
            "INTRODUCTION. This Article discusses the Restatement of the Law and American Law Institute materials as evidence, not as the document type.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_repository_law_review_articles_confidently() {
        let profile = classify(
            "1-1-2011",
            "University of Miami Law School Institutional Repository. University of Miami Inter-American Law Review. Recommended Citation. This Note is brought to you for free and open access. INTRODUCTION.",
            vec![
                block(LiquidBlockRole::Title, "1-1-2011"),
                block(
                    LiquidBlockRole::Heading,
                    "Nearshore Alternative: Latin America's Potential",
                ),
                block(
                    LiquidBlockRole::Footnote,
                    "1. See 42 U. Miami Inter-Am. L. Rev. 367.",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
        assert!(profile.confidence >= 0.65);
    }

    #[test]
    fn classifies_researchgate_legal_articles_as_academic_articles() {
        let profile = classify(
            "Measuring Transparency in Consumer Contracts",
            "See discussions, stats, and author profiles for this publication at: https://www.researchgate.net/publication/348232559. Article · October 2020. CITATIONS 1 READS 102. European Court of Justice contract terms.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Measuring Transparency in Consumer Contracts",
                ),
                block(LiquidBlockRole::Metadata, "Article · October 2020"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::ScienceArticle);
        assert!(profile.confidence >= 0.55);
    }

    #[test]
    fn classifies_scanned_documents_when_no_pages_have_text() {
        let profile = classify_document_profile(DocumentProfileInput {
            title: "Admissions Stats",
            source_text: "",
            blocks: &[],
            page_count: 4,
            extracted_pages: 0,
        });

        assert_eq!(profile.kind, DocumentProfileKind::ScannedImageOnly);
        assert!(profile.confidence > 0.9);
    }

    #[test]
    fn classifies_sparse_multi_page_text_as_scanned_for_ocr() {
        let profile = classify_document_profile(DocumentProfileInput {
            title: "Resume",
            source_text: "YONATHAN A. ARBEL\n\n2\n\n3\n\n4\n\n5",
            blocks: &[],
            page_count: 8,
            extracted_pages: 5,
        });

        assert_eq!(profile.kind, DocumentProfileKind::ScannedImageOnly);
        assert!(profile.confidence >= 0.9);
    }

    #[test]
    fn classifies_letters_as_free_prose() {
        let profile = classify(
            "Letter to the Yale Law Journal Forum",
            "Dear Yale Law Journal Forum, I never thought it would happen to me. I am writing about scholarship.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Letter to the Yale Law Journal Forum",
                ),
                block(LiquidBlockRole::Paragraph, "Dear Yale Law Journal Forum,"),
                block(
                    LiquidBlockRole::Paragraph,
                    "I never thought it would happen to me.",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FreeProse);
    }

    #[test]
    fn classifies_footnoted_law_review_forum_letters_as_law_review() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Title,
                "Letter to the Yale Law Journal Forum",
            ),
            block(LiquidBlockRole::Paragraph, "Dear Yale Law Journal Forum,"),
        ];
        for index in 0..6 {
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                &format!("This paragraph develops the forum essay argument {index}."),
            ));
        }
        for index in 0..8 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{index} See Example, 100 Yale L. J. 1 (2020)."),
            ));
        }

        let profile = classify(
            "Letter to the Yale Law Journal Forum",
            "Dear Yale Law Journal Forum, I am writing about legal scholarship. Sincerely, Professor. Yale Law Journal Forum. 1 See Example, 100 Yale L. J. 1 (2020).",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::LawReviewArticle);
    }

    #[test]
    fn classifies_formal_expert_letters_as_free_prose() {
        let profile = classify(
            "Tuscaloosa, Alabama, 35406",
            "U.S. Citizenship and Immigration Services. Dear USCIS Officer, I am writing this letter to offer my expert professional opinion on a contractual relationship.",
            vec![
                block(LiquidBlockRole::Title, "Tuscaloosa, Alabama, 35406"),
                block(LiquidBlockRole::Lead, "Dear USCIS Officer,"),
                block(
                    LiquidBlockRole::Paragraph,
                    "I am writing this letter to offer my expert professional opinion.",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FreeProse);
    }

    #[test]
    fn classifies_formal_expert_letters_despite_law_review_publications() {
        let profile = classify(
            "Tuscaloosa, Alabama, 35406",
            "U.S. Citizenship and Immigration Services. Dear USCIS Officer, I am writing this letter to offer my expert professional opinion. I have published in the NYU Law Review and Washington University Law Review.",
            vec![
                block(LiquidBlockRole::Title, "Tuscaloosa, Alabama, 35406"),
                block(LiquidBlockRole::Lead, "Dear USCIS Officer,"),
                block(
                    LiquidBlockRole::Paragraph,
                    "I am writing this letter to offer my expert professional opinion.",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FreeProse);
        assert!(profile.confidence >= 0.6);
    }

    #[test]
    fn classifies_cvs_as_academic_packets_despite_publication_and_case_markers() {
        let mut blocks = vec![block(
            LiquidBlockRole::Title,
            "Academic Resume, September 2026",
        )];
        for index in 1..=10 {
            blocks.push(block(
                LiquidBlockRole::Footnote,
                &format!("{index}. Article, 137 Harv. L. Rev. 1058; Smith v. Jones."),
            ));
        }
        let profile = classify(
            "Academic Resume, September 2026",
            "Curriculum vitae. Education. Publications. Selected works include articles in law reviews and cases such as Smith v. Jones.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
    }

    #[test]
    fn classifies_practice_academic_cvs_despite_legal_markers() {
        let profile = classify(
            "Jonathan G. Odom, JD, LLM, USN (Ret.)",
            "Professional Strengths. Teaching, Research, and Service Interests. Education. Teaching Experience. A recognized expert who has published 10 journal articles and 11 book chapters. Lecturer in Law. Contract Law. U.S. government. Supreme Court. Smith v. Jones.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Jonathan G. Odom, JD, LLM, USN (Ret.)",
                ),
                block(LiquidBlockRole::Heading, "Professional Strengths"),
                block(LiquidBlockRole::Heading, "Education"),
                block(LiquidBlockRole::Heading, "Teaching Experience"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_list_heavy_credentialed_cvs_confidently() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Title,
                "Jonathan G. Odom, JD, LLM, USN (Ret.)",
            ),
            block(LiquidBlockRole::Heading, "Professional Strengths"),
            block(LiquidBlockRole::Heading, "Education"),
            block(LiquidBlockRole::Heading, "Teaching Experience"),
            block(LiquidBlockRole::Heading, "Publications"),
        ];
        for index in 1..=36 {
            blocks.push(block(
                LiquidBlockRole::ListItem,
                &format!(
                    "Professional record {index}: teaching, publications, service, and law-school work."
                ),
            ));
        }
        let profile = classify(
            "Jonathan G. Odom, JD, LLM, USN (Ret.)",
            "Professional Strengths. Teaching, Research, and Service Interests. Education. Teaching Experience. Publications. A recognized expert who has published journal articles and book chapters. Contract Law. Supreme Court. Smith v. Jones.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_receipts_as_financial_documents() {
        let profile = classify(
            "Alabama expenses - House Hunting Trip",
            "Receipt. Payment method: Visa. Subtotal $90.00. Total $105.00. Transaction approved.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Alabama expenses - House Hunting Trip",
                ),
                block(LiquidBlockRole::Marginalia, "Total $105.00"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::ReceiptInvoiceFinancial);
    }

    #[test]
    fn classifies_event_logistics_packets_as_general_documents() {
        let profile = classify(
            "Inaugural AI Law Safety Roundtable",
            "Dear Participants. Logistics. Accommodation Options. Booking Deadline. Schedule Highlights. Have your email address and payment method ready for hotel booking. Contact information for logistical issues.",
            vec![
                block(LiquidBlockRole::Title, "Inaugural AI Law Safety Roundtable"),
                block(LiquidBlockRole::Heading, "1. Logistics"),
                block(LiquidBlockRole::Heading, "2. Accommodation Options"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::GeneralDocument);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_expense_reports_as_financial_documents() {
        let profile = classify(
            "Stanford Law and Economics Seminar",
            "Project Expense Report. Spend by Category. Expense Register. Date Vendor Description Category Amount. Receipt Evidence.",
            vec![
                block(LiquidBlockRole::Title, "Stanford Law and Economics Seminar"),
                block(LiquidBlockRole::Heading, "Expense Register"),
                block(LiquidBlockRole::KeyClause, "Date Vendor Category Amount"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::ReceiptInvoiceFinancial);
        assert!(profile.confidence >= 0.7);
    }

    #[test]
    fn classifies_scanned_travel_receipts_as_financial_documents() {
        let profile = classify(
            "2017/03/23 - 07:48 PM",
            "Departing Flight Information. American Airlines. Flight 1833, Philadelphia Intl Airport (PHL) to Hartsfield-Jackson Atlanta Intl Airport. Aircraft Embraer 190. Payment total amount.",
            vec![
                block(LiquidBlockRole::Title, "2017/03/23 - 07:48 PM"),
                block(LiquidBlockRole::Heading, "Departing Flight Information"),
                block(LiquidBlockRole::Marginalia, "American Airlines"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::ReceiptInvoiceFinancial);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_course_materials_separately() {
        let profile = classify(
            "Contracts Assignment 12",
            "Anonymous number. Assignment due before class meeting. The final exam will use the same rules.",
            vec![
                block(LiquidBlockRole::Title, "Contracts Assignment 12"),
                block(LiquidBlockRole::Paragraph, "Anonymous number."),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CourseOrExamMaterial);
    }

    #[test]
    fn classifies_syllabi_from_body_markers_when_title_is_course_name() {
        let profile = classify(
            "SECURED TRANSACTIONS",
            "SYLLABUS. Course Materials. Assignments. Problem Set 1. Casebook reading.",
            vec![
                block(LiquidBlockRole::Title, "SECURED TRANSACTIONS"),
                block(LiquidBlockRole::Syllabus, "SYLLABUS"),
                block(LiquidBlockRole::Heading, "Course Materials"),
                block(LiquidBlockRole::Heading, "Assignments"),
                block(LiquidBlockRole::Table, "• Problem Set 1"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CourseOrExamMaterial);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_course_evaluations_as_course_materials() {
        let profile = classify(
            "2021",
            "Course: LAW211 A - Civil Procedure II. Responsible Faculty: JoNel Newman. Responses / Expected: 20 / 56. Overall Mean: 4.0. Survey Comparisons.",
            vec![
                block(LiquidBlockRole::Title, "2021"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Course: LAW211 A - Civil Procedure II",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CourseOrExamMaterial);
    }

    #[test]
    fn classifies_eval_title_as_course_material() {
        let profile = classify(
            "Spring 2023 Sec Reg Evals",
            "Course: LAW520 Securities Regulation. Responses / Expected: 20 / 40. Survey Comparisons.",
            vec![
                block(LiquidBlockRole::Title, "Spring 2023 Sec Reg Evals"),
                block(LiquidBlockRole::Heading, "Survey Comparisons"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CourseOrExamMaterial);
    }

    #[test]
    fn classifies_course_eval_body_despite_bad_extracted_title() {
        let profile = classify(
            "Neither Agree nor Disagree (3) 0 0.00%",
            "The class sessions helped me learn the course content. Strongly Disagree (1) 0 0.00%. Response Rate Mean STD Median School Mean STD Median. The exams reflected the learning outcomes listed on the syllabus.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Neither Agree nor Disagree (3) 0 0.00%",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "Response Rate Mean STD Median School Mean STD Median",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CourseOrExamMaterial);
    }

    #[test]
    fn classifies_westlaw_result_lists_as_general_documents() {
        let profile = classify(
            "2025 100 Ind. L.J. 1431 Ignacio Cofone",
            "For Educational Use Only. List of 41 results for adv: Eric A. Stewart. Thomson Reuters. No claim to original U.S. Government Works. 1 Citing Reference.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "2025 100 Ind. L.J. 1431 Ignacio Cofone",
                ),
                block(LiquidBlockRole::Heading, "List of 41 results for adv"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::GeneralDocument);
        assert!(profile.confidence >= 0.8);
    }

    #[test]
    fn classifies_chat_exports_as_general_documents() {
        let profile = classify(
            "How people actually make these figures",
            "Thought for 16s. Great question. Here's what people typically use. Building method diagrams https://chatgpt.com/c/69114970-0eb0-832d-9cff-5314b25b0465 1 of 20.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "How people actually make these figures",
                ),
                block(LiquidBlockRole::Header, "https://chatgpt.com/c/69114970"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::GeneralDocument);
    }

    #[test]
    fn classifies_codebooks_as_general_documents() {
        let profile = classify(
            "Last Updated: 8.17.23",
            "Draft Codebook - Nondisclosure / Confidentiality Agreements. Each variable to be coded is identified in a table. plaintiff Description: Name of plaintiff. Type: Text string. Format: Full name. Notes: Imported from CNC dataset. defendant Description: Name of defendant. Type: Text string. Format: Full name. Notes: Imported from CNC dataset.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Draft Codebook - Nondisclosure / Confidentiality Agreements",
                ),
                block(LiquidBlockRole::Heading, "plaintiff"),
                block(LiquidBlockRole::Heading, "Description: Name of plaintiff"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::GeneralDocument);
    }

    #[test]
    fn classifies_property_deal_sheets_as_general_documents() {
        let profile = classify(
            "Join Our Whatsapp Community to Get First Access to Our Off Market Deals!",
            "Below is our current inventory of direct/cash deals (Off Market). All deals are Cash or Hard Money only. Price: $815,000. ARV: $1.3m. Beds: 4. Baths: 2. Sqft: 1,938. Year Built: 1925. Lot Size: 5,401 Sq Ft. Occupancy: Vacant.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Join Our Whatsapp Community to Get First Access to Our Off Market Deals!",
                ),
                block(LiquidBlockRole::Heading, "Price: $815,000"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::GeneralDocument);
    }

    #[test]
    fn classifies_cv_from_academic_body_even_with_law_school_text() {
        let profile = classify(
            "Director of Program on Ethics",
            "Notre Dame Law School. Academic Appointments. Selected Publications. Courses Taught. Professional Service. Bar Admissions.",
            vec![
                block(LiquidBlockRole::Title, "Director of Program on Ethics"),
                block(LiquidBlockRole::Heading, "Academic Appointments"),
                block(LiquidBlockRole::Heading, "Selected Publications"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
    }

    #[test]
    fn classifies_academic_position_resumes_despite_law_review_publications() {
        let profile = classify(
            "Academic Position",
            "Academic Position. Education. Works in Progress. Legal and Practice Experience. Publications in law reviews. Professional Service.",
            vec![
                block(LiquidBlockRole::Title, "Academic Position"),
                block(LiquidBlockRole::Heading, "Academic Position"),
                block(LiquidBlockRole::Heading, "Works in Progress"),
                block(LiquidBlockRole::Heading, "Legal and Practice Experience"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
        assert!(profile.confidence >= 0.6);
    }

    #[test]
    fn classifies_faculty_application_packets_as_academic_packets() {
        let profile = classify(
            "Application: Christian Johnson",
            "\
Application: Christian Johnson
Form: Faculty Application
Required Documents
Resume / Curriculum
Vitae
Cover Letter
Teaching Philosophy
CHRISTIAN A. JOHNSON
PRESENT POSITION & PRIOR ACADEMIC EMPLOYMENT
Professor of Law
Publications
Journal of Law and Business, 42 Law Review 100 (2021).",
            vec![
                block(LiquidBlockRole::Title, "Application: Christian Johnson"),
                block(LiquidBlockRole::Heading, "Form: Faculty Application"),
                block(LiquidBlockRole::Heading, "Required Documents"),
                block(LiquidBlockRole::Heading, "Resume / Curriculum Vitae"),
                block(
                    LiquidBlockRole::Heading,
                    "PRESENT POSITION & PRIOR ACADEMIC EMPLOYMENT",
                ),
                block(LiquidBlockRole::Heading, "Publications"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
        assert!(profile.confidence >= 0.8, "{profile:?}");
    }

    #[test]
    fn classifies_heading_heavy_cv_even_with_law_review_citations() {
        let mut blocks = vec![
            block(LiquidBlockRole::Title, "Yonathan A. Arbel"),
            block(LiquidBlockRole::Heading, "Current Appointment"),
            block(LiquidBlockRole::Heading, "Selected Publications"),
            block(LiquidBlockRole::Heading, "Professional Service"),
        ];
        for index in 1..=9 {
            blocks.push(block(
                LiquidBlockRole::Footnote,
                &format!("{index}. Journal article, 137 Harv. L. Rev. 1058."),
            ));
        }
        let profile = classify(
            "Yonathan A. Arbel",
            "Current Appointment. Education. Publications. Selected Publications. Teaching: Contracts. Professional Service. Bar Admissions. Law Review.",
            blocks,
        );

        assert_eq!(profile.kind, DocumentProfileKind::CvOrAcademicPacket);
    }

    #[test]
    fn classifies_tax_returns_as_financial_documents() {
        let profile = classify(
            "AL 2024",
            "Alabama Individual Income Tax Return. Federal return. Adjusted gross income. Taxable income. Withholding. Refund due.",
            vec![
                block(LiquidBlockRole::Title, "AL 2024"),
                block(LiquidBlockRole::Marginalia, "Adjusted gross income: $100"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::ReceiptInvoiceFinancial);
    }

    #[test]
    fn classifies_restatements_as_book_or_chapter_material() {
        let profile = classify(
            "Restatement (Second) of Agency Section 1",
            "Restatement (Second) of Agency. American Law Institute. Section 1. Agency; Principal; Agent.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Restatement (Second) of Agency Section 1",
                ),
                block(LiquidBlockRole::Heading, "Section 1"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::BookOrChapter);
        assert!(profile.confidence >= 0.7, "{profile:?}");
    }

    #[test]
    fn classifies_scanned_manual_pages_as_book_material() {
        let profile = classify(
            "Thomas Hartman Guide To The National Electrical Code 2005",
            "General Rules for Installation Part I. Table 7-7. Rules for Underground Installations. Application Rule Code Ref. Minimum burial depth.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Thomas Hartman Guide To The National Electrical Code 2005",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "Table 7-7. Rules for Underground Installations",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::BookOrChapter);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_language_manual_front_matter_as_book_material() {
        let profile = classify(
            "Pearson Education Limited",
            "Preface to the first edition. This book has been designed to meet the requirements of students whose mother tongue is not English. Pearson Education Limited. Longman-ELT. This manual is not exhaustive.",
            vec![
                block(LiquidBlockRole::Title, "Pearson Education Limited"),
                block(LiquidBlockRole::Heading, "Preface"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::BookOrChapter);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_palgrave_chapters_as_book_material_not_filings() {
        let profile = classify(
            "Shavell Palgrave Causation In Tort Law",
            "Causation and tort liability. Green, E.J. and Porter, R.H. 1984. References. Bibliography. Palgrave.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Shavell Palgrave Causation In Tort Law",
                ),
                block(LiquidBlockRole::Paragraph, "Causation and tort liability."),
                block(LiquidBlockRole::Heading, "References"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::BookOrChapter);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_blog_printouts_as_news_articles() {
        let profile = classify(
            "Positng On A National Blog",
            "prawfsblawg.blogs.com/prawfsblawg/2016/07/hiring-committees-2016-2017.html. Posted by: Anon. Comments. Permalink.",
            vec![
                block(LiquidBlockRole::Title, "Positng On A National Blog"),
                block(LiquidBlockRole::Lead, "prawfsblawg.blogs.com"),
                block(LiquidBlockRole::Paragraph, "Posted by: Anon"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::NewsArticle);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_teaching_philosophy_as_free_prose() {
        let profile = classify(
            "Teaching Philosophy",
            "Teaching Philosophy. The degree to which students are able to learn depends on academic preparation, motivation, critical thinking, and the teacher. School of Law. Research and writing.",
            vec![
                block(LiquidBlockRole::Title, "Teaching Philosophy"),
                block(
                    LiquidBlockRole::Paragraph,
                    "The degree to which students learn...",
                ),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FreeProse);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_uscis_rfe_notices_as_admin_documents() {
        let profile = classify(
            "U.S. Department of Homeland Security",
            "U.S. Citizenship and Immigration Services. Request for Evidence. I-129 Petition for a Nonimmigrant Worker. This notice contains your unique receipt number.",
            vec![
                block(
                    LiquidBlockRole::Title,
                    "U.S. Department of Homeland Security",
                ),
                block(LiquidBlockRole::Heading, "REQUEST FOR EVIDENCE"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FormReceiptAdmin);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_state_business_filings_as_admin_documents() {
        let profile = classify(
            "Active Officers",
            "State of Alabama. Domestic Nonprofit Corporation. Certificate of Amendment. Secretary of State Business Services. Entity ID Number. Certificate of Incorporation.",
            vec![
                block(LiquidBlockRole::Title, "Active Officers"),
                block(LiquidBlockRole::Heading, "CERTIFICATE OF AMENDMENT"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::FormReceiptAdmin);
        assert!(profile.confidence >= 0.6, "{profile:?}");
    }

    #[test]
    fn classifies_ali_restatement_drafts_as_book_material() {
        let profile = classify(
            "Restatement Consumer Contracts",
            "As of the date of publication, this Draft has not been considered by the members of The American Law Institute. SUBJECTS COVERED. Restatement of the Law Consumer Contracts. Tentative Draft. Standard Contract Terms.",
            vec![
                block(LiquidBlockRole::Title, "Restatement Consumer Contracts"),
                block(LiquidBlockRole::Heading, "SUBJECTS COVERED"),
                block(LiquidBlockRole::Heading, "§ 1. Definitions and Scope"),
            ],
        );

        assert_eq!(profile.kind, DocumentProfileKind::BookOrChapter);
        assert!(profile.confidence >= 0.7, "{profile:?}");
    }
}
