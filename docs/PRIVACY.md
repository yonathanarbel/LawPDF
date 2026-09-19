# Privacy and local data

LawPDF reads and annotates PDFs locally. An ordinary open, search, highlight, or save does not upload the document. There is no automatic document-content telemetry. This policy describes the desktop production candidate and the Android persistence changes; older releases can behave differently.

## When information leaves the device

- Desktop update checks contact GitHub and disclose the connection's IP address and LawPDF's version in its user-agent. Downloads contact GitHub's artifact delivery infrastructure. Document contents and API keys are not included.
- Asking a document chat question sends the question, conversation context, and selected document context to OpenRouter and its selected model provider.
- Choosing cloud OCR sends page images to OpenRouter and its selected vision provider. Operating-system OCR runs locally.
- Choosing a cloud-assisted reading feature sends the text needed for that feature to its named provider. Local Review Mode reconstruction uses bundled models on the device.
- Choosing paid narration sends the text to OpenAI or OpenRouter, according to the selected provider. System narration uses the operating system's speech service.
- The separate **Send review** action uploads generated review text, layout metadata, the document title, and the optional comment to `lawpdf.battleoftheforms.com` for improving reading reconstruction. The dialog explains this before submission. Do not use it for confidential material. Reading corrections stored locally are not automatically uploaded by this action unless included in the submitted review.
- Opening a hyperlink in a document contacts its destination in the user's browser.

Cloud providers have their own accounts, charges, retention settings, and privacy policies. Configuring a provider key does not itself upload a PDF. See the applicable [GitHub](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement), [OpenRouter](https://openrouter.ai/privacy), [OpenAI](https://openai.com/policies/privacy-policy/), and [Groq](https://groq.com/privacy-policy/) policies before choosing a cloud feature.

## What stays on the device

Desktop preferences, per-document zoom, OCR and reading caches, raster caches, local corrections, backups, and recovery records are stored in LawPDF's application-data folder. Windows raster caches use Local AppData. Document paths and content can appear in these local files. A recovery record includes annotations and an immutable copy of the PDF version they belong to. This allows recovery after the original moves or changes. Unsaved recovery data is preserved until recovered or explicitly discarded. The recovery source store has a 4 GB limit; reaching it produces a visible error rather than dropping edits.

API keys are stored in macOS Keychain or Windows Credential Manager. The supported Linux backend uses Secret Service. Legacy settings-file keys are removed only after the native credential store confirms migration. A failed migration leaves the existing file and displays an error. Old settings backups created by earlier versions may still contain keys; remove those backups after confirming that secure migration succeeded. Environment-supplied keys are read from the launching environment and are not written into settings.

Settings provides a retention period for cached content and old backup copies, and **Clear cached text and images** removes regenerable caches and unused recovery sources. It preserves active document sources, pending recovery records, preferences, and local corrections. A brief grace period protects files still opening. Cached content may be regenerated while documents remain open. Corrections and their local event log can be removed by deleting the `liquid-feedback` folder after quitting. Uninstalling the desktop executable does not delete document data or native-store credentials.

The support-diagnostics export contains only an explicit list of version, platform, job counts, and success/failure flags. It excludes document paths and text, keys, raw server responses, environment variables, and crash payloads. Nothing is sent automatically when creating this file.

Android keeps immutable PDF sources and annotation journals in private app storage. Completed edits are saved automatically there; the source supplied by the document provider is preserved. Reopening identical PDF bytes restores the marks. A changed PDF is treated as a different revision. If provider access is lost, an available verified local source can be opened as a recovery copy. The Android app excludes its storage from its own cloud/device backup rules. Clearing Android app storage or uninstalling removes those local copies and marks. Export a copy before doing so. Android **Save copy** creates flattened image pages; selectable text, editable annotation objects, and digital-signature validity are not preserved in that export.

## Removal and questions

To remove all desktop local data, first recover or export any pending edits, quit LawPDF, remove its application-data folder, and remove the `org.lawpdf.LawPDF` credential-store entry if desired. This does not delete PDFs stored elsewhere. Use the [repository](https://github.com/yonathanarbel/LawPDF) to contact the maintainer. Do not attach private documents, API keys, or recovery folders to a public issue.
