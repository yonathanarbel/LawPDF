# Microsoft Store distribution

This path uses an MSIX package that Microsoft signs after certification. It does
not submit the existing unsigned EXE installer as a Store download.

## Current status

The Individual developer account is created and verified, the Microsoft Developer
Agreement has been accepted, and the name LawPDF is reserved under public publisher
John Arbel. Microsoft assigned Store ID `9MZXSP0C423F` and package identity
`JohnArbel.LawPDF_byt1cn7x1mmx2`. The exact manifest identity is recorded in
`packaging/microsoft-store/identity.json`.

The submission is being prepared. No Microsoft certification, Store publication,
or fresh Store installation is claimed. The workflow builds the real unsigned
submission package using this identity; validation-only packages must not be
uploaded as the product.

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
- Store package version `1.2.34.0` maps to application version `0.2.34`. The package
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
