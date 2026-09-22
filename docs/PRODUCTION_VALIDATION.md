# LawPDF 0.2.32 initial candidate: historical local evidence

This record describes the initial local candidate and package identified below. Later changes, installation identities and GitHub results supersede its then-open gates; see [the follow-up](ACCESSIBILITY_FOLLOWUP.md) and the [current 0.2.33 release evidence](releases/v0.2.33.md) and [Mac lifecycle verification](reviews/2026-09-22-macos-lifecycle.md). In particular, the earlier submission block was resolved and the initial Mac package has since been replaced.

**Assessment date: September 17, 2026. Decision: suitable for controlled Mac testing; not yet qualified for broad production distribution.**

The reported marking, quit and update problems led to a substantial reliability change: automatic annotation saves now have a durable recovery journal and source-revision checks; shutdown waits for writes and prevents native-worker respawn; native PDF processing is supervised; updates require a pinned release signature and exact package hashes. Recovery, undo/redo, secure credential storage, Android persistence and release packaging were also addressed. Passing local checks does not establish the remaining platform and field requirements.

## What was verified

| Final check | Result and scope |
|---|---|
| Desktop regression suite | **1,080 passed**, development feature set including the normal suite, from the curated source. Covers annotations, save/close overlap, recovery, external revision conflicts, protected PDFs, rotated/cropped pages and parser behavior. |
| Native-process fault recovery | Built application rendered a synthetic PDF, recovered after its PDF worker was killed, rejected new requests after shutdown, and left the source unchanged. |
| Real Mac windows | **Six clean open/close cycles**, three with a PDF and three empty; all exited successfully. This does not cover every user interaction under editing/narration load. |
| Native credential integration | Write/read/clear/delete succeeded against a uniquely named, disposable Keychain service. The user's provider-key entry was not changed by this check. |
| Android build and unit tests | Clean candidate build and lint passed; **13 tests in debug and 13 in release**, no failures. |
| Android emulator | **Four real lifecycle/provider-copy checks passed.** A separate driver committed a mark, force-stopped the app, and restored it in a different process (PIDs 5095 and 5141). Its initial runner-registration failure was corrected and the failed check rerun. This is not evidence of every Storage Access Framework provider or physical device. |
| Markdown verification | **15 passed.** These regression fixtures are not a representative, held-out legal-text fidelity corpus. |
| Dependency audit | **Zero known vulnerabilities** against the refreshed local RustSec database. Maintenance warnings remain for `paste` and `ttf-parser`. |
| Static analysis | Completed with **zero errors and 406 warning diagnostics**, including duplicate diagnostics across targets. The optimized production build reports 30 warnings. Existing style/dead-code debt remains. |
| Mac package | Optimized 0.2.32 bundle, ad hoc signature verified, macOS 13 minimum declared, 399 target-specific Rust notice directories, current pinned PDFium, and complete bundled-model runtime checks passed. |
| Installed Mac app | `/Applications/LawPDF.app` reports both version fields as **0.2.32**. Runtime verification exited 0 with `requirements_met: true`, loading the installed runtime. Installation checked that LawPDF was closed. |

The main desktop suite and native QA used implementation commit `4290ae7a78dc5719cd9b008cc7939b152672d3ae`. The Android-only QA registration correction is commit `ae706e5`; it changes test configuration/driver code, not the production desktop implementation. Subsequent documentation records these results without changing executable code. Full logs and package evidence were kept outside the Box checkout under `/private/tmp/lawpdf-final-032-*`.

## Package identity and rollback

- Mac ZIP: `LawPDF-macos.zip`, **48,586,334 bytes**.
- ZIP SHA-256: `fa38c700e512a9a566e9f413350f2e233a4e2dac333b37297444d35422ac035b`.
- Installed executable SHA-256: `a65aab07b588b43a36a8c82335e9516f61f941baed4277e29cb7bc4b138ceede`.
- Previous 0.2.31 app retained at `/Applications/.LawPDF-before-production-0.2.32-20260917.app`.
- The Mac app is **ad hoc signed and not notarized**. No Windows publisher-signing approval is claimed.

## Narrow performance sample

On this Apple M4 Mac with 16 GiB RAM, the packaged native backend opened five existing article PDFs (22–79 pages; 0.21–1.84 MB) in **11.8–224.9 ms**. Twenty page renders, including repeated first pages, took **4.3–8.8 ms**. Peak child-process memory was **44.44 MiB**, and all original file hashes were unchanged. These measurements exclude application UI startup, journal/source capture, OCR, reading reconstruction and total app/model memory. They do not satisfy the broader p95 or held-out fidelity gates.

## Remaining production gates

1. **Clean GitHub CI and Windows installation.** A curated public source commit, workflows and exact-installer verification are prepared. GitHub source-tree creation was rejected by automatic approval review because the action exceeded its 200,000-byte review limit. No candidate branch or PR was published. Windows build/install/version/shortcut/runtime evidence therefore remains unavailable.
2. **Actual signed release/update qualification.** The pinned signing key and authentication implementation exist, but there is no published 0.2.32 release or completed cross-platform package attestation, signed-manifest, replacement/relaunch and interrupted-install exercise. The Mac updater keeps a previous bundle; LaunchServices accepting a launch is not proof of a healthy reader session.
3. **Accessibility.** Keyboard paths and control labels were improved. VoiceOver/NVDA task completion, especially PDF canvas and reconstructed reading order, still needs direct qualification.
4. **Fidelity and field reliability.** A representative private, held-out corpus, larger/scanned-document performance cohorts, cloud-provider conflict testing and a consented beta reliability sample remain necessary. Proposed acceptance criteria are in the production plan; they are not measured claims.
5. **Platform signing and Android release.** GitHub distribution is the owner's approved free fallback. SignPath requires a truthful application, contact address and MFA confirmation; approval is not available. Android also needs its separate production signing identity and physical-device qualification.
6. **Security boundary.** The PDF worker contains native crashes and enforces resource limits. It is not an operating-system security sandbox. Adversarial fuzzing and broader native-parser security qualification remain separate work.
7. **Previously published research.** The candidate excludes research/training files and development exports. Older GitHub history still contains files previously published there; this curation does not erase that history.

See [the production completion plan](PRODUCTION_PLAN.md) for acceptance criteria and [the release procedure](RELEASING.md) for artifact approval and signing. The original September 17 review remains a historical diagnosis; this record describes the implemented candidate and its actual evidence.
