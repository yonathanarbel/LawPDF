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
