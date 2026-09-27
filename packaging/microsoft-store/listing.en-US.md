# Microsoft Store listing

Publication status: app 0.2.34 is live under publisher John Arbel, Store ID
9MZXSP0C423F (verified September 27, 2026). This listing accompanies app
0.2.36 in Submission 2, submitted September 27, 2026. Partner Center confirmed
Update in certification (Pre-processing). The new Windows reading and coffee
screenshots, support-price disclosure, and external-commerce declaration were
saved before submission. Certification and a fresh Store installation on the
owner's Windows PC remain unverified.

## Product name
LawPDF

## Short description
Read, annotate, and follow the footnotes in legal scholarship.

## Description
Created by Professor Yonathan Arbel, LawPDF is a free, open-source PDF reader and annotation editor for legal research, teaching, and close reading.

Read the original PDF, search for a passage, highlight important text, leave comments, and add text boxes. Review Mode reconstructs reading flow and footnotes in law review articles while keeping the original PDF available for checking quotations, citations, and layout.

LawPDF saves annotations automatically and includes recovery copies, undo and redo, and visible save status. If a save fails, the document remains open so you can decide what to do next.

Ordinary PDF reading and annotation work locally. Optional cloud AI, OCR, and narration features require your own provider account and may incur provider charges. They send the selected content needed for the requested feature to the chosen service; review the privacy policy before using confidential material.

Optional coffee support: the small cup icon opens Buy Me a Coffee in your browser. Support starts at US$5 per coffee; choose a quantity and review the total before paying. Monthly support is optional. Support unlocks no features or rewards. Microsoft is not the fundraiser or payment provider.

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

The small coffee-cup icon in the top bar opens an optional support panel. Its
"Buy me a coffee" button opens https://buymeacoffee.com/lawpdf in the default
browser. Buy Me a Coffee handles voluntary support through its hosted payment
processor. No digital goods, features, memberships, or other rewards are provided
in exchange. Reading and editing require no payment. LawPDF does not collect
payment details or send document data through this link. Do not make a payment
while testing the app.

## Submission verification
- The maintainer's payout connection and public coffee form were verified on
  September 27, 2026. No test payment was made. Buy Me a Coffee was disclosed in Partner Center,
  and the external-commerce property was selected.
- Actual Windows screenshots of this build using a public or synthetic sample document.
- Completed age-rating questionnaire based on the shipped capabilities.
- Successful package installation evidence and any certification feedback.
