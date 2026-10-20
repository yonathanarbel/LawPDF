# Privacy and local data

LawPDF reads and annotates PDFs locally. An ordinary open, search, highlight, or save does not upload the document. There is no automatic document-content telemetry. This policy describes the desktop production candidate and the Android persistence changes; older releases can behave differently.

## When information leaves the device

- Direct-download desktop update checks contact GitHub and disclose the connection's IP address and LawPDF's version in its user-agent. Downloads contact GitHub's artifact delivery infrastructure. Document contents and API keys are not included. Microsoft Store builds disable these GitHub update checks and receive updates through Microsoft Store.
- Asking a document chat question sends the question, conversation context, and selected document context to OpenRouter and its selected model provider.
- Choosing cloud OCR sends page images to OpenRouter and its selected vision provider. On-device OCR (**Run OCR** and **OCR: save a searchable copy**) uses the operating system's own recognizer, Windows OCR or macOS Vision, and runs locally. A separately installed Tesseract is used only if the system recognizer is unavailable, and it also runs locally.
- Choosing a cloud-assisted reading feature sends the text needed for that feature to its named provider. Local Review Mode reconstruction uses bundled models on the device.
- **Find source** sends that footnote's text to the BBGPT search service at `bbgpt.battleoftheforms.com`, which queries bibliographic providers. **Find article links** queues up to 32 unresolved notes from the prepared Review. Neither action uploads the PDF. Existing URLs and DOI links are opened directly without a search request. Opening a document does not contact the citation service. Search results are candidates, not verified citation matches.
- Speech defaults to **System speech**, which uses the computer's installed speech engine and keeps document text local. Choosing **ElevenLabs** or **OpenRouter** and clicking **Read aloud** or **Create MP3** sends the article text to that provider (and, for OpenRouter, its selected model provider), using your key. Reading footnotes sends their text too when that option is selected. Provider charges apply. Stop ends LawPDF's playback and prevents later narration requests; a request already in progress may finish at the provider.
- The separate **Send review** action uploads generated review text, layout metadata, the document title, and the optional comment to `lawpdf.battleoftheforms.com` for improving reading reconstruction. The dialog explains this before submission. Do not use it for confidential material. Reading corrections stored locally are not automatically uploaded by this action unless included in the submitted review.
- Opening a hyperlink in a document contacts its destination in the user's browser.
- Choosing the optional coffee-support link opens [Buy Me a Coffee](https://buymeacoffee.com/lawpdf) in your browser. LawPDF does not collect payment details or attach document contents, filenames, API keys, or usage data to that link. Buy Me a Coffee and its payment processor handle the transaction under their own policies; see [Buy Me a Coffee's privacy policy](https://buymeacoffee.com/privacy-policy). Support is voluntary and does not unlock app features.

Cloud providers have their own accounts, charges, retention settings, and privacy policies. Configuring a provider key does not itself upload a PDF. See the applicable [GitHub](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement), [OpenRouter](https://openrouter.ai/privacy), [ElevenLabs](https://elevenlabs.io/privacy-policy), [OpenAI](https://openai.com/policies/privacy-policy/), and [Groq](https://groq.com/privacy-policy/) policies before choosing a cloud feature.

## What stays on the device

Desktop preferences, per-document zoom, OCR and reading caches, raster caches, local corrections, backups, and recovery records are stored in LawPDF's application-data folder. Windows raster caches use Local AppData. Document paths and content can appear in these local files. A recovery record includes annotations and an immutable copy of the PDF version they belong to. This allows recovery after the original moves or changes. Unsaved recovery data is preserved until recovered or explicitly discarded. The recovery source store has a 4 GB limit; reaching it produces a visible error rather than dropping edits.

API keys are stored in macOS Keychain or Windows Credential Manager. The supported Linux backend uses Secret Service. Legacy settings-file keys are removed only after the native credential store confirms migration. A failed migration leaves the existing file and displays an error. Old settings backups created by earlier versions may still contain keys; remove those backups after confirming that secure migration succeeded. Environment-supplied keys are read from the launching environment and are not written into settings.

Speech uses private temporary text and audio files in the application-data `speech-cache` folder. These files are removed when narration finishes or stops; leftovers from an interrupted session are eligible for automatic cleanup after a day. An MP3 you explicitly export stays at your chosen destination.

Settings provides a retention period for cached content and old backup copies, and **Clear cached text and images** removes regenerable caches and unused recovery sources. It preserves active document sources, pending recovery records, preferences, and local corrections. A brief grace period protects files still opening. Cached content may be regenerated while documents remain open. Corrections and their local event log can be removed by deleting the `liquid-feedback` folder after quitting. Uninstalling a direct-download desktop installation does not delete document data or native-store credentials.

The `citation-sources` cache stores bibliographic result metadata and a hash of the search query, not the input footnote text. Results expire after 14 days; complete searches with no results expire after one day. Partial or failed searches are not cached. The cache is bounded to 256 files and 16 MiB. **Search again** bypasses a cached result.

For Microsoft Store installations, Windows can remove package-private application data when the app is uninstalled or reset, including cached content and recovery copies. Save or export pending edits to ordinary document files before uninstalling or resetting. PDFs saved outside the package's application-data folders are separate from the installed app. Existing data from a direct-download installation and native-store credentials can remain. See [Microsoft's description of packaged desktop app storage](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes).

The support-diagnostics export contains only an explicit list of version, platform, job counts, and success/failure flags. It excludes document paths and text, keys, raw server responses, environment variables, and crash payloads. Nothing is sent automatically when creating this file.

Starting with desktop 0.2.40, the local `logs` folder also keeps lifecycle and
error records in `diagnostics.jsonl`, plus a session marker used to detect an
interrupted prior session. Records include timestamps, application version,
process IDs, shutdown stages, bounded error descriptions, and panic locations;
the application does not intentionally record document text or provider keys.
On macOS, it also records summaries of LawPDF crash reports (report filename,
OS/app versions, exception, and termination reason), not their complete contents.
These files are not uploaded automatically or included in the minimal support
export described above. On launch, a diagnostic log larger than 2 MiB is rotated,
retaining one previous log. After quitting, the `logs` folder and `panic.log` can
be removed without deleting document annotations or recovery copies. Inspect
diagnostic files for private information before sharing them.

Android keeps immutable PDF sources and annotation journals in private app storage. Completed edits are saved automatically there; the source supplied by the document provider is preserved. Reopening identical PDF bytes restores the marks. A changed PDF is treated as a different revision. If provider access is lost, an available verified local source can be opened as a recovery copy. The Android app excludes its storage from its own cloud/device backup rules. Clearing Android app storage or uninstalling removes those local copies and marks. Export a copy before doing so. Android **Save copy** creates flattened image pages; selectable text, editable annotation objects, and digital-signature validity are not preserved in that export.

## Removal and questions

To remove all desktop local data, first recover or export any pending edits, quit LawPDF, remove its application-data folder, and remove the `org.lawpdf.LawPDF` credential-store entry if desired. This does not delete PDFs stored elsewhere. Use the [repository](https://github.com/yonathanarbel/LawPDF) to contact the maintainer. Do not attach private documents, API keys, or recovery folders to a public issue.
