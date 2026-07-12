# Third-Party Notices

This file summarizes third-party software distributed with LawPDF or linked into
the Windows release build. It is a practical notice file, not legal advice.

## Rust Dependencies

Rust crate license metadata for the Windows target is generated in
`THIRD_PARTY_RUST_LICENSES.csv` from:

```powershell
cargo metadata --locked --filter-platform x86_64-pc-windows-msvc
```

The current Windows dependency set reports permissive licenses such as MIT,
Apache-2.0, BSD-family licenses, ISC, Zlib, Unicode-3.0, and compatible
multi-license expressions. The audit did not identify GPL, AGPL, or LGPL
licensed Rust crates in the Windows target dependency graph.

## PDFium Binary

LawPDF bundles `pdfium.dll` for Windows PDF rendering. The binary is from the
`bblanchon/pdfium-binaries` distribution. Its license file and the third-party
licenses included in that package are committed under:

```text
third_party/pdfium-binaries/
```

Release packages include this notice directory.

## EB Garamond

LawPDF bundles EB Garamond for the user interface. EB Garamond is licensed under
the SIL Open Font License 1.1. The OFL text is committed under:

```text
third_party/eb-garamond/OFL.txt
```

## Inno Setup

The Windows installer is built with Inno Setup and includes the Inno Setup setup
runtime. Its license is committed under:

```text
third_party/inno-setup/LICENSE.txt
```

## Tesseract OCR

LawPDF invokes `tesseract.exe` only when OCR is requested. Tesseract is not
bundled in the LawPDF release packages; users install it separately.

## CatBoost Model Evaluation Library

Review Mode bundles the CatBoost model evaluation library and a trained CBM
model. CatBoost is Copyright 2017–2026 YANDEX LLC and is distributed under the
Apache License 2.0. Release libraries are pinned to CatBoost 1.2.10 artifacts
published by the official `catboost/catboost` GitHub project. The Windows DLL
is checksum-verified by `scripts/fetch-catboost-windows.ps1` before packaging.

The Apache License 2.0 text is committed at:

```text
third_party/catboost/LICENSE
```
