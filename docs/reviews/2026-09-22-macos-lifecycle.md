# macOS lifecycle correction for 0.2.33

## Reproduced failure

On the installed public 0.2.32 bundle, opening a private test PDF in Finder while
LawPDF was fully closed launched an AppKit alert: “LawPDF cannot open files in
the PDF document format.” Dismissing it left an empty reader. Opening the same
file from Finder with LawPDF already running worked.

AppKit replaces the early Apple-event registration during launch. The previous
second registration ran in eframe's app creator, after the initial open event.
An observer now restores the handler at `NSApplicationWillFinishLaunchingNotification`,
before that event, while preserving winit's application delegate.

## Closing

Command-W previously closed a tab but did nothing on the resulting empty window.
It now closes the empty window. Command-Q and native Quit requests now use the
same cancellable save/close path as the window close button. Previously the
native menu called AppKit termination directly, bypassing that path. Handler,
notification-observer and menu-target lifetimes are explicitly cleaned up.

The user's intermittent close-error dialog was not reproduced in 0.2.32 during
this session. These changes fix confirmed lifecycle defects; they do not prove
the cause of every historical close error.

## Local candidate evidence

- macOS 26.6.2, Apple silicon; candidate installed at `/Applications/LawPDF.app`.
- 1,079 production tests and 1,083 development-feature tests passed.
- Bundle signature verification and required native/context runtime checks passed.
- Finder cold launch reproduced the old failure, then opened the PDF correctly
  with 0.2.33 on repeated attempts.
- Command-W closed the tab; a second Command-W exited the app. App inventory
  confirmed it stopped, with no error dialog observed.
- A text box was added and typed into, followed immediately by Command-Q.
  The app stopped cleanly. Finder reopened the saved PDF; independent PDF
  inspection confirmed all 72 pages and the exact typed text in a FreeText
  annotation. Only a disposable copy was edited.

Automated tests additionally cover failed-save cancellation and closing the
empty window. Full screen-reader, field-reliability and update-interruption
qualification remain separate gates. Public package checks are recorded with
the eventual tagged release, not inferred from this local development build.


## Optimized candidate and final public package

The optimized local 0.2.33 bundle also passed repeated Finder opening of two
PDFs, including a Unicode filename, without duplicate tabs. With a disposable
read-only PDF, Command-Q showed the unsaved-changes prompt; a failed Save kept
the document open; Cancel retained the edit; Command-W presented the tab-close
prompt. Explicit discard closed that disposable tab, and a second Command-W
closed the empty reader. The source test file and original document hashes
remained unchanged.

[Version 0.2.33](https://github.com/yonathanarbel/LawPDF/releases/tag/v0.2.33)
was published on September 22, 2026, at 23:08 UTC as the latest public release.
The [publication workflow](https://github.com/yonathanarbel/LawPDF/actions/runs/35795936709)
verified all package hashes, tagged-build provenance, successful release/desktop
checks, exact Windows installation evidence, and the signed update manifest.
The public page returned HTTP 200 without authentication. All seven assets were
downloaded anonymously; all three packages matched the checksum file and signed
manifest, whose Ed25519 signature was independently verified.

The exact public Mac ZIP was extracted and installed at
`/Applications/LawPDF.app`, preserving the previous local build for rollback.
Both bundle version fields read 0.2.33. Strict deep code-signature verification
passed; the installed executable matched the downloaded package. Required
native/context runtime verification exited 0 with `requirements_met: true`,
using assets inside the installed app.

- Public Mac ZIP SHA-256: `6d6d9efde842c3fed15b72a27cc634aa6fae9cf8b3693b267503eca2be3c4395`.
- Installed Mac executable SHA-256: `30e476b4feb8ec8fcb4c5f6f1c4829256a6485a6b64b7ba97b99763ccf7fcc20`.
- Public Windows installer SHA-256: `6e575e79c5305f4d5d03845389f78e99609a3e438a7012a69a3c63a18ab3356b`.
- Public Windows portable SHA-256: `f2022ee3998120db0a808995dbdfd402e44171d2ea9eeb72b78124e960d4fd5e`.

The Mac desktop locked after the local candidate's hands-on checks. Therefore,
GUI retesting of the final GitHub-built package and live update/restart were not
completed. Installation, signature, executable identity, and runtime checks of
that exact package were completed without GUI access. Windows installation was
verified on the clean hosted Windows runner, not the owner's Windows computer.
This release remains a public beta; the broader qualification gates in the
[release notes](../releases/v0.2.33.md) remain open.

## October 15 follow-up: shutdown crash remains open

The 0.2.40 local lifecycle log captured two ordinary Command-Q sessions on macOS
26.6.2 that reached `shutdown_complete` but never reached `run_native_returned`
or `session_finished`. The subsequent launches recorded interrupted sessions
and imported summaries of the corresponding macOS `EXC_CRASH` / `SIGABRT`
reports. The exception backtrace passed through KVO observer removal and
`_NSTouchBarFinderObservation invalidate` in AppKit while returning through the
native event loop. This reproduces a native teardown failure after the PDF
worker shutdown completed, rather than a Rust panic or worker shutdown error.

The existing `NSFunctionBarAPIEnabled` default did not prevent these crashes.
Durable logging is verified; this is not a verified crash fix. The public-source
0.2.40 candidate (`3423d0ad877f0d5f145a08fd8f609f19976a9606`) was subsequently
installed for testing, with matching bundle versions, strict signature check,
and required native/context runtime exit code 0. No clean-shutdown qualification
or broad Mac release approval is inferred from that installation.

## October 15 follow-up: 0.2.41 shutdown correction

The curated 0.2.40 installation reproduced the same AppKit abort again during
an ordinary Command-Q from an empty reader. The macOS report identified the
same Touch Bar/KVO teardown path, after LawPDF's worker and settings cleanup.

Version 0.2.41 uses eframe's supported `run_and_return: false` mode on macOS.
The framework still saves and destroys the application and runs `on_exit`, then
exits without returning through the failing AppKit display-cycle cleanup. Other
platforms retain their existing event-loop mode. Save/close cancellation and
pending annotation saves are unchanged.

A normal-exit hook records `session_finished` with `clean_native_exit`, but only
after application cleanup completed. Cleanup alone does not remove the active
session marker. A native abort, forced quit or process kill therefore remains
detectable on the next launch; the fix does not relabel these as clean exits.

Local verification of source `289960e0ce46b679e86b796fedcf2e3c2cad1110`:

- All 1,118 development-feature tests passed, including the new exit-marker test
  and existing failed-save/cancel-close tests.
- Six isolated native-window close cycles passed: three empty readers and three
  readers given a private test PDF. All six recorded `clean_native_exit`, with no
  interrupted-session event. The PDF's SHA-256 was unchanged.
- An intentionally killed isolated test process retained its marker. Its next
  launch recorded the killed process as interrupted, then closed cleanly and
  removed the marker.
- The development-only shutdown test can isolate its data directory and compare
  both event-loop modes. These environment overrides are not in production.

The production bundle from that source was installed at
`/Applications/LawPDF.app`, with the previous installation preserved for rollback.
Both bundle versions are 0.2.41. Strict deep signature verification passed, and
the installed executable matches the packaged executable. Required native,
context, arbiter, note-head and link-ranker runtime verification exited 0 with
`requirements_met: true`, using the installed bundle's assets.

Two ordinary Command-Q tests of the installed app passed: one from an empty
reader and one after opening a disposable copy of the 21-page reading PDF.
Both recorded completed worker cleanup and `clean_native_exit`, left no running
app/worker or active-session marker, and produced no new macOS crash report.
The disposable PDF's hash remained unchanged. The first launch also imported
the preceding 0.2.40 crash report, verifying that the diagnostics still expose
native failures rather than hiding them.

- Local Mac ZIP SHA-256: `e011799c708bfb0c673d9889e677c6f9fe5b2066c57422319f913dd7d65ac9d0`.
- Installed Mac executable SHA-256: `18cab97374466029048b1cfab9efab8748d092a96d95a73f652075f013931edb`.
- [Desktop verification](https://github.com/yonathanarbel/LawPDF/actions/runs/38777731224)
  passed production/development tests on Mac (1,114/1,118) and Windows
  (1,111/1,115), plus dependency audit.
- [Release build](https://github.com/yonathanarbel/LawPDF/actions/runs/38777726750)
  passed both platforms. The exact Windows installer was installed on the hosted
  Windows runner: product/file version 0.2.41, runtime exit 0,
  `requirements_met: true`, and the Start Menu target verified. Its SHA-256 was
  checked again after download:
  `6e749640f9fa3d12b56356d29968a26958a5723d2e42c30e4caa70e08c982d27`.
  This was not an installation on the owner's Windows computer. The direct
  installer is not Authenticode-signed.

The Mac package, Windows installer and local verification evidence are retained
under `Downloads/LawPDF-0.2.41`. This branch build was not published as a tagged
GitHub release. The separate Microsoft Store 0.2.40 submission remains the
already-submitted Windows update; 0.2.41 does not replace it in certification.

This is a targeted correction for the reproduced teardown failure, not a claim
that every possible native crash or interruption has been eliminated.
