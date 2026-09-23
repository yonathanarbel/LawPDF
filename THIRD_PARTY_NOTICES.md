# Third-Party Notices

This file summarizes third-party software distributed with LawPDF or linked into
the desktop release builds. It is a practical notice file, not legal advice.

## Rust Dependencies

Each desktop package contains a target-specific `THIRD_PARTY_RUST_LICENSES.csv`
and a `rust-licenses/` directory copied from the locked upstream crates by
`tools/package_rust_notices.py`. The inventory uses `cargo metadata --locked`
with the package's target platform. The repository's older Windows inventory
is retained as historical reference; packaging regenerates it after dependency
changes.

Some upstream crate archives omit their workspace license files. Matching
upstream notices are vendored in `third_party/rust-extra/` with pinned source
revisions and SHA-256 hashes. Packaging includes those notices and their
provenance, and fails if a required crate has no license text.
For four older crates without an upstream license file, the supplement includes
the upstream license declaration/author metadata and the unmodified standard
SPDX license text; the provenance identifies this explicitly.

## PDFium Binaries

Mac and Windows builds bundle the matching `chromium/8057` release from
[bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium/8057),
published September 14, 2026. Both downloaded archives were checked against the
publisher's SHA-256 digests. The exact library hashes are recorded in
`release-manifest.json` and checked before packaging. The upstream license files
and dependency notices are distributed under `third_party/pdfium-binaries/`
(Windows) and `third_party/pdfium-binaries-mac-arm64/` (Mac).

## EB Garamond

LawPDF bundles EB Garamond for the user interface. EB Garamond is licensed under
the SIL Open Font License 1.1. The OFL text is committed under:

```text
third_party/eb-garamond/OFL.txt
```

## Frank Ruhl Libre

The masthead title is set in Frank Ruhl Libre Black, Copyright 2015 The Frank
Ruhl Libre Project Authors, licensed under the SIL Open Font License 1.1. LawPDF
embeds a static Black instance subset to Latin characters
(`vendor/fonts/FrankRuhlLibre-Black.ttf`), made from the variable font published
in [google/fonts](https://github.com/google/fonts/tree/main/ofl/frankruhllibre).
The family declares no Reserved Font Name, so the instanced subset keeps its
name. The OFL text is committed under:

```text
third_party/frank-ruhl-libre/OFL.txt
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
