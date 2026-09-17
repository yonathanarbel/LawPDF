# Production completion plan

The September 17 review is being implemented as desktop candidate **0.2.32**. Broad release remains gated on evidence for the exact source commit and packages. Major test suites belong at the end of implementation and release preparation; earlier checks are reserved for concrete blockers. The historical 0.2.31 results do not certify this candidate.

## Implementation status

| Area | Candidate implementation | Remaining qualification |
|---|---|---|
| Annotation durability | Durable journals, immutable source revisions, guarded atomic writes, conflict recovery, visible save states, and close waiting | Final persistence/fault matrix on the candidate; actual cloud-provider races and disk exhaustion need platform evidence |
| Editing and responsiveness | Annotation undo/redo; asynchronous open, Save, Save As, rotation and exports; bounded queues; closed-tab speculative work cancellation | An in-flight native call has a two-minute deadline; cancellation does not abort a write whose result is still needed |
| Native failures | PDFium runs in a supervised child, with bounded protocol/input/raster sizes and restart after failure; Mac and Windows PDFium are pinned to chromium/8057 | This is crash containment, not an OS security sandbox; adversarial fuzzing and long-running field reliability remain separate gates |
| Privacy | Native credential stores with verified legacy migration; retention controls; private recovery files; redacted diagnostics and panic logs; documented data flows | Exercise native-store failure/migration with disposable credentials; inspect provider and device-specific behavior |
| Updates | Pinned Ed25519 release key, exact manifest/size/hash checks, authenticated staging, retained rollback bundle, minimum-OS and runtime preflight | Verify actual packages, restart and interrupted-install paths; signature/provenance must refer to the exact candidate |
| Android | Private immutable sources, AtomicFile mark journals, rollback on failed persistence, lifecycle restoration, bounded rendering and explicit flattened-export notice | Emulator process-death/provider coverage, production signing identity, and physical-device accessibility remain separate from desktop approval |
| Accessibility | Native accessibility integration, labels on custom toolbar controls, keyboard save/find/undo/redo and close-modal handling | VoiceOver and NVDA task completion is not established by source inspection or unit tests |
| Release hygiene | Restored repository controls; explicit public source/model allowlist; private development exports removed; pinned Rust, Gradle and CI actions; platform license inventories | Clean-source Mac/Windows CI, exact installed-version checks and retained evidence |
| Maintainability | Extracted persistence, document-session, opening, recovery, close, process-supervision and geometry boundaries; removed future-incompatible inference warnings | Remaining style/dead-code warnings and large reading-reconstruction modules need incremental work |

## General availability acceptance gates

1. **No document loss:** zero lost/corrupted committed marks or changed originals in edit/reopen, close/save overlap, forced termination, external revision conflict, protected/read-only document and write-failure scenarios. Recoverable failures must produce a usable copy and an accurate status.
2. **Exact artifact verification:** both platforms build the curated commit. Install the Windows installer; verify SHA-256, product/file versions, Start Menu target, and successful `--lm2-runtime-status --require-native --require-context`. Mac verification includes the actual bundle, signing status, native child restart and real window shutdown.
3. **Update trust:** reject unsigned, altered, wrong-platform, stale and substituted releases. Check the actual downloaded package and test replacement/restart/recovery using disposable installations. Keep build provenance and the signed manifest with the release.
4. **Accessibility:** complete open, navigate, search, mark, undo, save, recover and close using keyboard plus VoiceOver/NVDA. PDF canvas accessibility and reconstructed reading order require human task evidence.
5. **Reading fidelity:** use a private, held-out corpus spanning court documents, contracts, articles, scans, tables, multilingual pages and unusual footnotes. Record missing/reordered substantive text and citation errors against the original. No missing substantive text or changed citations is acceptable in the audited release sample; uncertain reconstructions must keep the source available. Corpus data and development tooling stay outside the public repository.
6. **Performance and reliability:** before a controlled beta, record reference hardware and document sizes, then measure p95 open/navigation/save latency and peak memory. Suggested starting budgets are 3 s to the first usable page for text PDFs under 20 MB, 150 ms for a cached page, and 2 s for saving a 20 MB PDF; larger and scanned files need separate cohorts. These are proposed budgets, not measured claims. Broad release additionally needs at least 99.9% crash-free sessions in a representative, consented beta sample and zero confirmed unrecoverable annotation loss.
7. **Android qualification:** run real activity recreation and process-death restoration, provider-access loss and export checks. A desktop release does not certify Android.

## Free distribution and external dependencies

The owner has no Apple Developer ID, Windows publisher-signing credentials or Windows machine. Paid enrollment is outside scope. GitHub Releases is the accepted free distribution route; GitHub-hosted Windows CI supplies a disposable installation target. Ad hoc Mac signatures and application-level update signatures do not replace platform-recognized publisher identity.

Apple's individual free account does not provide Developer ID/notarization. Organization fee waivers require an eligible legal entity and authority to enroll it. SignPath Foundation offers free signing for qualifying open-source Windows projects, subject to application and review. The maintainer contact address and confirmation of repository MFA are still needed before a truthful application can be submitted. No approval is claimed.

## Evidence handling

Record the candidate commit, toolchain and dependency/native-runtime versions, test results, package hashes, installation results and unresolved gates in `PRODUCTION_VALIDATION.md`. Keep full logs in external build/evidence storage and attach the appropriate records to the candidate. Completed local tests are evidence for their tested snapshot; they are not a substitute for clean CI, accessibility, corpus fidelity, or field reliability.
