import unittest

from tools import md_verify


def report_for(markdown: str) -> tuple[list[md_verify.Block], md_verify.DocumentReport]:
    blocks = md_verify.split_blocks(markdown.splitlines())
    return blocks, md_verify.DocumentReport(path="fixture.md")


class FootnoteChecksTest(unittest.TestCase):
    def test_only_unreferenced_numeric_definitions_are_critical(self) -> None:
        blocks, report = report_for(
            "Body text.[^1]\n\n"
            "## Notes\n\n"
            "[^1]: Linked note.\n\n"
            "[^2]: Unlinked numeric note.\n\n"
            "[^author]: Author biography.\n\n"
            "[^*]: Author disclosure.\n"
        )

        md_verify.check_footnotes(blocks, report)

        orphans = [
            defect.detail
            for defect in report.critical
            if defect.kind == "footnote.orphan_definition"
        ]
        self.assertEqual(orphans, ["note 2 is defined but never referenced in the body"])

    def test_literal_continuation_notice_is_detected_outside_code(self) -> None:
        blocks, report = report_for(
            "Body.\n\n"
            "[^1]: FOOTNOTE CONTINUED ON NEXT PAGE\n\n"
            "```text\nfootnote continued on next page\n```\n"
        )

        md_verify.check_pagination_artifacts(blocks, report)

        defects = [
            defect
            for defect in report.critical
            if defect.kind == "furniture.footnote_continued"
        ]
        self.assertEqual(len(defects), 1)
        self.assertEqual(defects[0].line, 3)


class BlankBoundaryChecksTest(unittest.TestCase):
    def check(self, markdown: str) -> list[md_verify.Defect]:
        blocks, report = report_for(markdown)
        md_verify.check_blank_boundary_continuations(blocks, report)
        return report.critical

    def check_report(self, markdown: str) -> md_verify.DocumentReport:
        blocks, report = report_for(markdown)
        md_verify.check_blank_boundary_continuations(blocks, report)
        return report

    def test_open_paragraph_followed_by_lowercase_is_critical(self) -> None:
        defects = self.check(
            "# Title\n\n"
            "This sufficiently long body paragraph visibly ends in the middle of a\n\n"
            "lowercase continuation on the next extracted row.\n"
        )
        self.assertEqual([defect.kind for defect in defects], [
            "paragraph.open_lowercase_boundary"
        ])

    def test_open_paragraph_followed_by_uppercase_is_warning_only(self) -> None:
        report = self.check_report(
            "# Title\n\n"
            "This sufficiently long body paragraph visibly ends in the middle of a\n\n"
            "New continuation begins with an uppercase first line.\n"
        )
        self.assertFalse(report.critical)
        self.assertEqual(
            [defect.kind for defect in report.warnings],
            ["paragraph.open_uppercase_boundary"],
        )
        self.assertEqual(report.stats["open_uppercase_boundaries"], 1)

    def test_short_marker_ending_block_followed_by_it_is_warning_only(self) -> None:
        report = self.check_report(
            "# Title\n\n"
            "The organization employs secretaries, teachers, and maintenance crews.[^1]\n\n"
            "It does not require most employees to share its beliefs.\n"
        )
        self.assertFalse(report.critical)
        self.assertEqual(
            [defect.kind for defect in report.warnings],
            ["paragraph.open_uppercase_boundary"],
        )
        self.assertIn("short marker-ending paragraph", report.warnings[0].detail)

    def test_long_marker_ending_paragraph_is_not_treated_as_a_splice(self) -> None:
        report = self.check_report(
            "# Title\n\n"
            + " ".join(["Substantive"] * 31)
            + ".[\u005e1]\n\n"
            "It begins a genuinely separate paragraph.\n"
        )
        self.assertFalse(report.critical)
        self.assertFalse(report.warnings)

    def test_complete_sentence_boundaries_are_allowed(self) -> None:
        self.assertFalse(
            self.check(
                "# Title\n\n"
                "This sufficiently long body paragraph ends as a complete sentence.\n\n"
                "lowercase text may begin a genuinely separate quoted thought.\n"
            )
        )
        self.assertFalse(
            self.check(
                "# Title\n\n"
                "This sufficiently long body paragraph ends as a complete sentence.\u201d\n\n"
                "lowercase text begins after a closed quotation.\n"
            )
        )

    def test_structural_blocks_and_notes_are_excluded(self) -> None:
        fixtures = [
            "# Title\n\nThis sufficiently long paragraph introduces a list of\n\n- lower item\n",
            "# Title\n\nThis sufficiently long paragraph introduces a quotation from\n\n> lowercase quotation\n",
            "# Title\n\nThis sufficiently long paragraph introduces tabular data in\n\n| lowercase | value |\n",
            "# Title\n\nThis sufficiently long paragraph introduces sample code in\n\n```text\nlowercase code\n```\n",
            "# Title\n\nThis sufficiently long paragraph introduces sample code in\n\n~~~text\nlowercase code\n~~~\n",
            "# Title\n\nThis sufficiently long paragraph introduces the explanatory note in\n\n[^1]: lowercase note text\n",
            "# Title\n\n## Notes\n\nunlinked note tail without punctuation\n\nlowercase continuation\n",
        ]
        for fixture in fixtures:
            with self.subTest(fixture=fixture):
                self.assertFalse(self.check(fixture))

    def test_uppercase_warning_uses_the_same_structural_exclusions(self) -> None:
        fixtures = [
            "# Title\n\nThis sufficiently long paragraph introduces a list of\n\n- Upper item\n",
            "# Title\n\nThis sufficiently long paragraph introduces a quotation from\n\n> Uppercase quotation\n",
            "# Title\n\nThis sufficiently long paragraph introduces tabular data in\n\n| Uppercase | value |\n",
            "# Title\n\nThis sufficiently long paragraph introduces sample code in\n\n```text\nUppercase code\n```\n",
            "# Title\n\nThis sufficiently long paragraph introduces sample code in\n\n~~~text\nUppercase code\n~~~\n",
            "# Title\n\nThis sufficiently long paragraph introduces the explanatory note in\n\n[^1]: Uppercase note text\n",
            "# Title\n\n## Notes\n\nunlinked note tail without punctuation\n\nUppercase continuation\n",
        ]
        for fixture in fixtures:
            with self.subTest(fixture=fixture):
                report = self.check_report(fixture)
                self.assertFalse(report.critical)
                self.assertFalse(report.warnings)


class HeadingChecksTest(unittest.TestCase):
    def test_case_names_are_not_citation_prose_or_fused_enumerators(self) -> None:
        blocks, report = report_for(
            "# Bankruptcy v. Multidistrict Litigation for Mass Torts\n\n"
            "## 1. Harris v. Nelson\n\n"
            "## 2. Party Presentation Ignored: Erie Railroad Co. v. Tompkins\n"
        )

        md_verify.check_headings(blocks, report)

        self.assertFalse(
            [
                defect
                for defect in report.critical
                if defect.kind in {"heading.citation_prose", "heading.fused"}
            ]
        )

    def test_long_enumerated_title_is_warning_not_critical(self) -> None:
        blocks, report = report_for(
            "# Article Title\n\n"
            "## B. The Best Option for Originalists: Invocation of the Party "
            "Presentation Principle Should Remain Available in Carefully "
            "Defined Cases and Controversies\n"
        )

        md_verify.check_headings(blocks, report)

        overlong = [
            defect for defect in report.defects if defect.kind == "heading.overlong"
        ]
        self.assertEqual(len(overlong), 1)
        self.assertEqual(overlong[0].severity, md_verify.WARNING)


if __name__ == "__main__":
    unittest.main()
