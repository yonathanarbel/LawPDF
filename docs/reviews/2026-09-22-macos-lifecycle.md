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
