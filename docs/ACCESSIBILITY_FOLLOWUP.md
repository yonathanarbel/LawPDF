# Native accessibility follow-up to the 0.2.32 release candidate

This document records follow-up work after the initial candidate `eeff7e311b7d39dc2211dda2b2033ffdd6fc869b`. That snapshot was submitted with permission as GitHub commit `e700c3724d9725bf63afe92aa84961baa07a89ad` in [draft PR #21](https://github.com/yonathanarbel/LawPDF/pull/21), with an exactly matching source tree. Subsequent PR revisions supersede the original package and CI status; the PR records current validation against its head. Earlier identities below describe their stated checks, not the newest installed package.

Direct inspection of the installed Mac application's native accessibility tree found that custom-painted PDF pages exposed no document text. Marker and comment swatches lacked names, the page-number entry lacked a label, and tab-close controls were announced only as “x.”

The follow-up exposes visible PDF native text, or already available OCR text when native text is empty, in the normal reader, source review pane, and side-by-side panes. Extraction remains asynchronous and is requested only when an accessibility client is active. Results are associated with their document generation so tab changes cannot attach text to the wrong document. Swatches report names and selected state; document tabs report full titles and selected state; close buttons and the page-number field have descriptive names.

This is page-level reading access. It does not implement a full accessible text-selection model, tagged-PDF reading order, or prove end-to-end VoiceOver/NVDA task completion. Those remain production acceptance gates. No new model-generated text or document upload is involved.

## Final verification

- All **1,080 desktop regression tests passed** with the development feature set after the application changes were complete. The development application built successfully.
- A separate native Mac QA bundle exposed the synthetic source text in the accessibility tree: “Page 1 of 1. LawPDF production persistence and lifecycle QA.” A second document exposed its different text, and both texts appeared correctly in side-by-side panes.
- Marker controls reported Yellow, Pink, Mint, Blue, Lavender, and Crimson underline with on/off state. Activating Pink through accessibility changed the selected state and status; Yellow was then restored.
- The page entry reported “Page number”; full document titles and their close buttons were named, including the long second-document title that is visually shortened in the tab.
- In the installed original 0.2.32 bundle, a synthetic highlight created through the actual marking interface survived closing and reopening without pressing Save. Undo and redo worked before closing. This is a narrow interactive persistence check, not a general field-reliability claim.
- Native QA bundle executable SHA-256: `7f57b74c6caac21d1b81968eda13490ddc688bfa9f8be90af9d023b032a65338`. This separate development bundle was not installed over `/Applications/LawPDF.app`.

Logs remain outside the synced checkout at `/private/tmp/lawpdf-accessibility-final-tests.log` and `/private/tmp/lawpdf-accessibility-final-build.log`. The earlier production validation document describes the earlier installed package.

## GitHub workflow correction

The first submission revealed a workflow validation failure: `runner.temp` is not available in job-level `env`. The audit job now uses an explicit temporary build path, as allowed by [GitHub's context availability reference](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts#context-availability). Superseded pull-request package runs are cancelled; tagged release runs retain their existing serialization behavior. The original Android GitHub test/lint/package job passed (run `35283542181`). Desktop and exact-installer results must be recorded against the final PR head before release.


## Windows recovery correction found in final CI

The first complete Windows debug suite compiled successfully but failed 10 tests with access-denied errors. They traced to refreshing a deduplicated recovery snapshot through a read-only `File` handle. Windows [requires write-attributes access for timestamp changes](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfiletime). Reopening an already captured PDF could therefore fail to establish recovery, and a repeated saved revision could fail before writing.

The private snapshot is now opened with write access and no create/truncate option before refreshing its timestamp. Source PDF contents and permissions are not modified by that operation. A dedicated regression captures the same source again, checks unchanged bytes and revision, and verifies that the renewed snapshot survives retention. The retention fixture also requests the access needed to set its test timestamp. Final CI must pass with this correction before the Windows release gate is closed.


## Windows license-packaging correction

The corrected application passed all clean desktop suites: Mac 1,077 production / 1,081 development; Windows 1,076 production / 1,080 development, plus the Markdown checks and dependency audit. The optimized Windows suite also passed 1,076 tests and its release executable built. Packaging then failed before installer creation because Python decoded Cargo's UTF-8 JSON with Windows CP1252. The license collector now explicitly reads UTF-8, including its supplemental manifest. The public-source exporter also explicitly reads UTF-8 and includes this approved follow-up evidence document. Application Rust code and runtime assets are unchanged by this packaging correction. A completed exact-installer check is still required; passing application tests alone does not certify packaging.

The corrected license collector was also run locally with Python's preferred encoding forced to **US-ASCII** (`LC_ALL=C`, UTF-8 mode disabled). It successfully packaged all **399** Mac-target dependencies from the actual Cargo metadata; this directly exercises the non-UTF-8 locale failure mode. The Windows installer pipeline must still finish with this correction.


Windows checkout preservation
-----------------------------
The next final packaging run reached the license integrity check and rejected an
upstream AccessKit license. Git's automatic Windows line-ending conversion had
changed the extensionless license text. The supplemental upstream license tree
now disables text conversion so each checkout preserves the exact bytes recorded
in its SHA-256 manifest. The package integrity check remains unchanged.

Final targeted verification used two fresh checkouts with `core.autocrlf=true`.
The previous revision changed 22 of the 36 supplemental license files; the
corrected checkout matched all 36 recorded hashes. This exercises the checkout
behavior that caused the Windows failure without weakening the integrity check.


## Windows installation diagnostic collection

The final Windows run `35288312687` built both packages, passed all 1,076 optimized tests, packaged all 397 dependency notices and passed both portable bundled-runtime checks. The installer completed and its product/file versions and Start Menu targets passed. The verifier then read an empty runtime result before the GUI-subsystem executable finished; the script failed and its closing output pipe caused a later stdout error from the child.

The installation verifier now explicitly starts and waits for that exact process, captures stdout/stderr to evidence files, checks its exit code, and only then parses the runtime JSON. It retains the same required runtime checks and prints the non-secret installation evidence after success. This follows [Microsoft's documented Start-Process wait and output-redirection behavior](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process). A successful rerun is still required; the earlier installed-runtime result is not counted as a pass.

The final local Mac package was installed after the user's session closed. The installed executable SHA-256 is `440b3f80395bced6f939223cf5e47fc64805687ed805be388355dbbf2d8ea681`; both version fields are 0.2.32, bundle integrity and all required runtime checks passed, and the previous copy is retained at `/Applications/.LawPDF-before-final-0.2.32-20260917.app`. The installed interface reopened the synthetic saved highlight, exposed its page text through accessibility and quit with its worker. This local bundle is ad hoc signed and not notarized; it is not a downloaded CI artifact.
