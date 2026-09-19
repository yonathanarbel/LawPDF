# Code signing policy

Maintainer, author, reviewer, and release approver: [Yonathan Arbel](https://github.com/yonathanarbel), owner of the [LawPDF repository](https://github.com/yonathanarbel/LawPDF). Outside contributions require review before release. Signing approval follows review of the source commit, clean CI, exact artifact checksums, and installation evidence. CI prepares a draft; it does not publish an unreviewed release or replace already published artifacts.

## Current distribution

GitHub Releases is the official download channel. The macOS bundle is ad hoc signed, without Apple Developer ID signing or notarization. Windows publisher signing is not yet available. Neither an ad hoc Mac signature nor a release checksum establishes a platform-recognized publisher identity. These limitations must appear in release notes and installation instructions.

The project is preparing an application for free Windows signing through [SignPath Foundation](https://signpath.org/). Acceptance, account setup, and approval are still pending. LawPDF does not currently claim that SignPath signs its binaries. If accepted, this policy will identify the approved SignPath configuration and attribution before signed artifacts are distributed. Repository and signing-account multi-factor authentication are required by SignPath; the application must not assert compliance without confirmation.

Apple's individual free account does not provide Developer ID or notarization. An eligible legal entity may qualify for a membership fee waiver; the maintainer's employment by a university does not establish authority to enroll that university. No paid enrollment is part of the present release plan.

## Authenticated in-app updates

The application pins the public key in `packaging/update-public-key.hex`. A release's exact `UPDATE-MANIFEST.json` bytes must have a valid Ed25519 signature in `UPDATE-MANIFEST.sig`. The signature binds the repository, version, source commit, artifact names, byte sizes, and SHA-256 hashes. Only strictly newer supported versions can be installed. Verification occurs before downloading/installing and again when using staged updates. Unsigned historical releases are offered through a manual GitHub download link.

The private update-signing key lives in the maintainer's macOS Keychain and is not committed, placed in CI artifacts, or printed by the signing utility. This is application-level update authentication; it does not grant Apple notarization or Windows Authenticode status. GitHub build attestations provide separate provenance evidence. Losing the private key requires an explicit trust-key transition or manual installation of a new trusted application; do not silently generate a replacement key for an existing public key.

See [Privacy and local data](PRIVACY.md) for network behavior and [Releasing](RELEASING.md) for the release procedure.
