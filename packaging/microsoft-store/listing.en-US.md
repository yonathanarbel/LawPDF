# Microsoft Store listing draft

Publication status: not submitted. Account verification, the reserved product identity,
Store screenshots, certification and a fresh Store installation are still required.

## Product name
LawPDF

## Short description
Read, annotate, and follow the footnotes in legal scholarship.

## Description
Created by Professor Yonathan Arbel, LawPDF is a free, open-source PDF reader and annotation editor for legal research, teaching, and close reading.

Read the original PDF, search for a passage, highlight important text, leave comments, and add text boxes. Review Mode reconstructs reading flow and footnotes in law review articles while keeping the original PDF available for checking quotations, citations, and layout.

LawPDF saves annotations automatically and includes recovery copies, undo and redo, and visible save status. If a save fails, the document remains open so you can decide what to do next.

Ordinary PDF reading and annotation work locally. Optional cloud AI, OCR, and narration features require your own provider account and may incur provider charges. They send the selected content needed for the requested feature to the chosen service; review the privacy policy before using confidential material.

This is a public beta. Verify reconstructed text against the original PDF and retain backups of important documents. LawPDF is provided under the MIT License, without warranty of any kind.

## Features
- Read multiple PDFs in tabs, with search, text selection, zoom and continuous viewing.
- Highlight, underline, comment, and add text boxes to PDF documents.
- Reconstruct legal reading flow and footnotes with local Review Mode models.
- Save annotations automatically, with recovery copies and clear save failures.
- Receive application updates through Microsoft Store.

## Suggested category
Productivity

## Privacy URL
https://github.com/yonathanarbel/LawPDF/blob/main/docs/PRIVACY.md

## Support URL
https://github.com/yonathanarbel/LawPDF/issues

## Website
https://github.com/yonathanarbel/LawPDF

## Price
Free. Optional third-party cloud providers set their own charges.

## Restricted capability explanation for Microsoft reviewers
LawPDF is a native Rust/Win32 desktop PDF application, so it requires runFullTrust. It opens PDF files selected by the user, writes annotations to those files, stores recovery data in the user's application-data directory, stores optional provider keys in Windows Credential Manager, and runs its own child executable for PDF processing and native speech. It does not require administrator privileges. The Store build disables the direct-download updater; Microsoft Store owns package updates.

## Reviewer instructions
No LawPDF account or payment is required. Start LawPDF, choose Open PDF, and select a nonconfidential PDF. Read and search the PDF, add an annotation, wait for saved status, close the tab, and reopen the PDF to verify persistence. Review Mode uses the bundled models. Optional network-backed AI features require a user-supplied provider account; they are not needed to test reading, annotation, saving, or local Review Mode. Use only disposable documents during certification tests.

## Submission details still needed
- Reserved name and exact Package/Identity/Name, Publisher, and publisher display name from Partner Center.
- Actual Windows screenshots of this build using a public or synthetic sample document.
- Completed age-rating questionnaire based on the shipped capabilities.
- Successful package installation evidence and any certification feedback.
