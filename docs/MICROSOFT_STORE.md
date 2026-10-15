# Microsoft Store distribution

This path uses an MSIX package that Microsoft signs after certification. It does
not submit the existing unsigned EXE installer as a Store download.

## Current status

The Individual developer account is created and verified, the Microsoft Developer
Agreement has been accepted, and the name LawPDF is reserved under public publisher
John Arbel. Microsoft assigned Store ID `9MZXSP0C423F` and package identity
`JohnArbel.LawPDF_byt1cn7x1mmx2`. The exact manifest identity is recorded in
`packaging/microsoft-store/identity.json`.

The first submission (package `1.2.34.0`, app 0.2.34) was verified as published
on September 27, 2026. Submission 2 (`1152921505701986313`, package `1.2.36.0`,
app 0.2.36) was submitted that day with the revised reading interface, optional
coffee-support panel, and matching screenshots. Partner Center last showed
**Update in certification** for that submission, with automatic publication
after approval. These are dated observations, not a claim about today's Store
version. On October 15, 2026, the signed-in Partner Center dashboard confirmed
submission 2 was live with package `1.2.36.0`.

Package `1.2.38.0` contains app 0.2.38 and adds Save As in the Windows toolbar
and on Ctrl+Shift+S.
On October 12, 2026, the [Store verification workflow](https://github.com/yonathanarbel/LawPDF/actions/runs/38260970593)
passed for source `37db5c01d4e129473e0d429f0796b51bdf9d37e5`. The exact unsigned
submission package SHA-256 is
`6ce0c0d07c974041d44f10ba8937e49faa4e2deecd98daeca4582ff1cd76ac4d`.
Its test-signed copy passed installation, identity, product/file version,
Start Menu registration, PDF association, and required-runtime checks on the
hosted runner. The original unsigned package was downloaded and its package
and executable hashes were independently checked.

That submission was prepared but awaiting Partner Center sign-in. Version 0.2.38 has
not been submitted or certified as part of this release. Store availability
is separate from the [direct-download release](https://github.com/yonathanarbel/LawPDF/releases/tag/v0.2.38).
A fresh installation through Microsoft Store on the owner's Windows PC has not
been verified. Validation-only packages must not be uploaded as the product.

On October 15, 2026, the packaging identity was advanced to `1.2.40.0` for app
0.2.40. This update preserves the Store-specific update and speech behavior and
adds serialized Review preview/full-document preparation plus durable local
lifecycle diagnostics. Interrupted sessions are reported at the next launch;
force quit cannot execute an exit handler. Windows package verification and
Partner Center submission must be confirmed for this exact source before this
version is described as available in Microsoft Store.

On October 15, the [Store verification workflow](https://github.com/yonathanarbel/LawPDF/actions/runs/38772527411)
passed for source `3423d0ad877f0d5f145a08fd8f609f19976a9606`.
The downloaded unsigned package SHA-256 is
`8c196adcfcbea36ad32d01d887b16d66981ac71c75d33dc68711ce66603f343e`;
its executable SHA-256 is
`fcffb497a7b9eeec276e86056ee01a0893016d9cf060fb1771029a04c1914ed8`.
Both hashes were independently checked after download. The test-signed copy
passed exact installation, product/file version, Start Menu, PDF association,
Store-channel behavior and required-runtime checks on the disposable Windows
runner. The direct-installer [release build](https://github.com/yonathanarbel/LawPDF/actions/runs/38772531944)
also passed its exact-install checks; neither result is an installation on the
owner's Windows hardware.

Partner Center validated the unsigned `1.2.40.0` package in submission 3
(`1152921505702141171`). With the owner's approval, the package replacement was
saved to the draft. This is **not** a submission for certification or publication;
the live package remains `1.2.36.0` until a later confirmed release.

## Build and packaging

- Build with `--features microsoft-store`. This disables GitHub update checks,
  staged direct updates, and external update-helper execution. Settings explains
  that Microsoft Store manages updates.
- Windows read-aloud uses SAPI from the application's own child process rather
  than launching PowerShell. Speech text is treated as plain text, not markup.
- The package registers `.pdf` support and a Start Menu app. Default-app selection
  remains under the user's control in Windows Settings.
- `scripts/package-microsoft-store.ps1` requires the exact Partner Center identity,
  a Store-enabled portable payload, and an output directory outside the checkout.
  It generates the manifest/icons, uses Windows SDK MakeAppx validation, unpacks
  the result, and checks the executable and manifest identity.
- The package depends on Microsoft's VC++ Desktop runtime framework; Store
  deployment supplies that dependency. Minimum OS is Windows 10 build 19041.
- Store package version `1.2.40.0` maps to application version `0.2.40`. The package
  major is nonzero and the fourth component is zero, as Microsoft requires. This
  packaging identity does not change the application's public-beta status.

## Verification boundary

The Store workflow runs the Store regression suite and builds an unsigned MSIX.
On a disposable GitHub-hosted Windows runner only, it signs a **copy** with a
temporary test certificate, installs it, verifies package identity, executable
hash/product/file versions, Start Menu registration, PDF association and required
native/context runtime. The test package and certificate are removed afterward.
Only the original unsigned package and evidence are retained for submission.

A passing CI install is not Microsoft certification or a real Store installation.
Before calling the Store release complete, download it through Microsoft Store
on an ordinary Windows machine, verify that no security override is required,
and check launch, PDF activation, annotation saving and Store update behavior.

## Submission

Use the real product identity from Partner Center in `identity.json`, rerun the
workflow, and upload the resulting `store-submission.msix`. Listing text and
reviewer instructions are in `packaging/microsoft-store/listing.en-US.md`.
Screenshots must show the real Windows app with public or synthetic documents.
Do not upload confidential research or client files. Keep the listing unpublished
until Microsoft has accepted the account, package and submission.

References: [Microsoft's MSIX packaging guide](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[package preparation](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare),
[Store package requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements),
and [Store signing](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options).
