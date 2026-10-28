# Linux

LawPDF's first Linux package targets x86_64 on Ubuntu 24.04 or a compatible
modern glibc desktop. It uses the same Review Mode models, annotations, speech
preferences, and document pipeline as the other desktop builds. Linux support
was adapted from Sarah Lawsky's Linux port; see `CONTRIBUTORS.md`.

## Use a release package

Download `LawPDF-linux-x86_64.tar.gz` and verify it against the release's
`SHA256SUMS.txt` before extracting it. Keep the extracted directory together:
`lawpdf` loads the bundled `libpdfium.so` and `profile-models/` beside itself.
Launch `./lawpdf` or `./lawpdf /path/to/document.pdf`.

Run `./install.sh` to add this location to your application menu. It writes only
to your XDG data directory (normally `~/.local/share`), needs no administrator
rights, and does not change your default reader. The in-app menu can make
LawPDF your default PDF reader using `xdg-mime`; LawPDF verifies the result.
Run `./uninstall.sh` from the same installation to remove its menu entry. It
will refuse to remove an entry registered by another LawPDF installation.
Choose another default reader in system settings if needed after uninstalling.

Linux updates are manual. Extract a newer release into a new directory and run
its `install.sh` before deleting the old directory.

## Optional system services

- **Printing:** install a CUPS client (`cups-client` on Ubuntu) and configure a
  printer in desktop settings. Ctrl+P prepares a private copy with current,
  including unsaved, annotations. The dialog lets you choose printer, copies,
  and page ranges. CUPS receives the copy only when you press Print. A queue
  receipt means accepted by the service; it does not prove paper was printed.
- **System speech:** install `espeak-ng` or `espeak`. Only LawPDF's synthesizer
  is stopped when you press Stop. API speech offers ElevenLabs and OpenRouter
  through the same preferences used on Mac and Windows. API playback needs
  `ffplay` (FFmpeg) or `mpg123`; MP3 export works without a player. API keys are
  stored in an unlocked Secret Service keyring, such as GNOME Keyring or KWallet.
- **OCR:** install `tesseract-ocr` and the language packages you need. Linux uses
  the Tesseract fallback; the release package does not redistribute Tesseract.
- **Default PDF reader:** desktop integration uses `xdg-utils`.

Printer discovery, PDF print export, speech, and desktop registration run only
when requested. They do not add service discovery to reading or app startup.
The existing model inference pipeline is unchanged.

## Build

Install Rust and these Ubuntu build dependencies:

```sh
sudo apt-get install libgl1-mesa-dev libwayland-dev libxkbcommon-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev libfontconfig1-dev \
  libdbus-1-dev libssl-dev python3 curl
export CARGO_TARGET_DIR=/tmp/lawpdf-target
./scripts/fetch-pdfium-linux.sh
./scripts/fetch-catboost-linux.sh
cargo build --release --locked
./scripts/package-linux.sh
```

The fetch scripts download pinned upstream releases, verify SHA-256 before
installing, and reuse an existing library only if its checksum matches. PDFium
is `chromium/8057`; CatBoost is `1.2.10`. `release-manifest.json` records the
library hashes, and the PDFium archive digest is pinned in its fetch script and
manifest. Upstream Linux PDFium licenses and notices are in
`third_party/pdfium-binaries-linux-x64/`.

Packaging writes to `/tmp/lawpdf-dist` by default (or `$TMPDIR/lawpdf-dist`). Set
`LAWPDF_DIST_DIR` for another location outside the checkout. The script rejects
build and distribution directories inside the checkout, including symlinks
into it. It stages only an explicit runtime allowlist, generates Rust license
notices, initializes the exact bundled PDFium, and verifies all promoted model
tiers from outside the package directory. CI also extracts the archive and
checks real OCR and desktop installation/removal.

To verify the model runtime manually:

```sh
./lawpdf --lm2-runtime-status --require-native --require-context \
  --require-arbiter --require-note-head --require-link-ranker
```
