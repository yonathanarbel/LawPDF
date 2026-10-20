#!/usr/bin/env bash
# Build the Linux x86_64 release tree and the distributable tarball.
#
# The layout matches what the application already probes at runtime: the
# executable sits beside `libpdfium.so` and the `profile-models` tree, so a
# user can extract the tarball anywhere and run `./lawpdf` without installing
# anything. `install.sh` inside the tarball adds the desktop entry and icon for
# users who want LawPDF in their application menu.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="lawpdf"
PROFILE="${LAWPDF_LINUX_PROFILE:-release}"
TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/lawpdf-target}"
DIST_DIR="${LAWPDF_DIST_DIR:-${TMPDIR:-/tmp}/lawpdf-dist}"
# Resolve overrides and reject generated trees inside the checkout, including
# symlinks into it. This source tree may be Box-synced.
TARGET_DIR="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve())' "$TARGET_DIR")"
DIST_DIR="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve())' "$DIST_DIR")"
export CARGO_TARGET_DIR="$TARGET_DIR"
python3 - "$ROOT" "$TARGET_DIR" "$DIST_DIR" <<'PYCODE'
from pathlib import Path
import sys
root = Path(sys.argv[1]).resolve()
for value in sys.argv[2:]:
    path = Path(value).resolve()
    if path == root or root in path.parents:
        raise SystemExit(f"Build and package output must stay outside the checkout: {path}")
PYCODE
if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "error: Build this package on Linux x86_64 (Ubuntu 24.04 or compatible)." >&2
  exit 1
fi
if [[ "$PROFILE" != "release" && "$PROFILE" != "debug" ]]; then
  echo "error: LAWPDF_LINUX_PROFILE must be release or debug." >&2
  exit 1
fi
BUILD_ARGS=(--locked)
VERSION="$(awk -F ' *= *' '$1 == "version" {gsub(/"/, "", $2); print $2; exit}' "$ROOT/Cargo.toml")"
STAGE="$DIST_DIR/LawPDF-linux-x86_64"

if [[ "$PROFILE" == "release" ]]; then
  BUILD_ARGS+=(--release)
fi

cd "$ROOT"

# The native CatBoost runtime is fetched rather than committed, matching Windows.
"$ROOT/scripts/fetch-catboost-linux.sh"
"$ROOT/scripts/fetch-pdfium-linux.sh"
python3 "$ROOT/tools/verify_release_manifest.py" --platform linux --require-platform-library

cargo build "${BUILD_ARGS[@]}"

BIN="$TARGET_DIR/$PROFILE/$APP_NAME"
if [[ ! -x "$BIN" ]]; then
  echo "error: expected a built executable at $BIN" >&2
  exit 1
fi

mkdir -p "$DIST_DIR"
rm -rf "$STAGE"
mkdir -p "$STAGE"

install -m 0755 "$BIN" "$STAGE/$APP_NAME"

install -m 0644 "$ROOT/vendor/libpdfium.so" "$STAGE/libpdfium.so"

install -m 0644 "$ROOT/assets/lawpdf.png" "$STAGE/lawpdf.png"
install -m 0644 "$ROOT/LICENSE" "$ROOT/THIRD_PARTY_NOTICES.md" "$STAGE/"
python3 "$ROOT/tools/package_rust_notices.py" --target x86_64-unknown-linux-gnu --destination "$STAGE"

mkdir -p "$STAGE/third_party"
for component in pdfium-binaries-linux-x64 eb-garamond frank-ruhl-libre catboost; do
  cp -R "$ROOT/third_party/$component" "$STAGE/third_party/$component"
done

# Release packages contain only the explicit runtime allowlist below. Private
# Python environments and training directories are never copied into the tree.

NATIVE_RUNTIME_SOURCE="$ROOT/profile-models/lm2-native-catboost-runtime"
FASTTAB_RUNTIME_SOURCE="$ROOT/profile-models/lm2-fasttab-runtime"
CONTEXT_RUNTIME_SOURCE="$ROOT/profile-models/lm2-context-twopass-runtime"
NOTE_HEAD_RUNTIME_SOURCE="$ROOT/profile-models/lm2-note-head-runtime"
LINK_RANKER_RUNTIME_SOURCE="$ROOT/profile-models/lm2-link-ranker-runtime"
FASTTAB_MODEL="fasttab-v1.onnx"
NATIVE_MODEL="lm2-catboost-augmented-epoch51lv-relabels-tc.cbm"
NATIVE_LIBRARY="libcatboostmodel-linux-x86_64-1.2.10.so"
CONTEXT_MODEL="lm2-context-twopass-hgb-v1.json"
CONTEXT_ARBITER_MODEL="lm2-runtime-residual-rescue-v7.json"
NOTE_HEAD_MODEL="lm2-note-head-hgb-v1.json"
LINK_RANKER_MODEL="lm2-footnote-link-ranker-hgb-v1.json"
for asset in \
  "$FASTTAB_RUNTIME_SOURCE/$FASTTAB_MODEL" \
  "$NATIVE_RUNTIME_SOURCE/$NATIVE_MODEL" \
  "$NATIVE_RUNTIME_SOURCE/$NATIVE_LIBRARY" \
  "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_MODEL" \
  "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_ARBITER_MODEL" \
  "$NOTE_HEAD_RUNTIME_SOURCE/$NOTE_HEAD_MODEL" \
  "$LINK_RANKER_RUNTIME_SOURCE/$LINK_RANKER_MODEL"; do
  if [[ ! -f "$asset" ]]; then
    echo "missing promoted LM2 runtime asset: $asset" >&2
    exit 1
  fi
done

NATIVE_RUNTIME_DEST="$STAGE/profile-models/lm2-native-catboost-runtime"
FASTTAB_RUNTIME_DEST="$STAGE/profile-models/lm2-fasttab-runtime"
CONTEXT_RUNTIME_DEST="$STAGE/profile-models/lm2-context-twopass-runtime"
NOTE_HEAD_RUNTIME_DEST="$STAGE/profile-models/lm2-note-head-runtime"
LINK_RANKER_RUNTIME_DEST="$STAGE/profile-models/lm2-link-ranker-runtime"
mkdir -p "$FASTTAB_RUNTIME_DEST" "$NATIVE_RUNTIME_DEST" "$CONTEXT_RUNTIME_DEST" \
  "$NOTE_HEAD_RUNTIME_DEST" "$LINK_RANKER_RUNTIME_DEST"
install -m 0644 "$FASTTAB_RUNTIME_SOURCE/$FASTTAB_MODEL" "$FASTTAB_RUNTIME_DEST/$FASTTAB_MODEL"
install -m 0644 "$NATIVE_RUNTIME_SOURCE/$NATIVE_MODEL" "$NATIVE_RUNTIME_DEST/$NATIVE_MODEL"
install -m 0644 "$NATIVE_RUNTIME_SOURCE/$NATIVE_LIBRARY" "$NATIVE_RUNTIME_DEST/$NATIVE_LIBRARY"
install -m 0644 "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_MODEL" "$CONTEXT_RUNTIME_DEST/$CONTEXT_MODEL"
install -m 0644 "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_ARBITER_MODEL" "$CONTEXT_RUNTIME_DEST/$CONTEXT_ARBITER_MODEL"
install -m 0644 "$NOTE_HEAD_RUNTIME_SOURCE/$NOTE_HEAD_MODEL" "$NOTE_HEAD_RUNTIME_DEST/$NOTE_HEAD_MODEL"
install -m 0644 "$LINK_RANKER_RUNTIME_SOURCE/$LINK_RANKER_MODEL" "$LINK_RANKER_RUNTIME_DEST/$LINK_RANKER_MODEL"
install -m 0644 "$ROOT/release-manifest.json" "$STAGE/release-manifest.json"

# Registration is implemented by the executable so paths have exactly the
# same escaping and validation as the in-app desktop integration action.
cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$HERE/lawpdf" --install-desktop-entry
echo "Added LawPDF to your application menu. Keep this extracted folder in place."
echo "To use it as your default PDF reader, choose Make default PDF reader in LawPDF."
INSTALL
cat > "$STAGE/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$HERE/lawpdf" --uninstall-desktop-entry
UNINSTALL
chmod 0755 "$STAGE/install.sh" "$STAGE/uninstall.sh"

cat > "$STAGE/README.txt" <<README
LawPDF $VERSION for Linux (x86_64)

Run ./lawpdf, or open a document with ./lawpdf path/to/file.pdf.
Run ./install.sh to add LawPDF to your application menu, and ./uninstall.sh to
remove it again. Keep the files in this directory together: lawpdf loads
libpdfium.so and profile-models/ from beside the executable.

Optional features:
  - OCR of scanned pages needs Tesseract on PATH (package: tesseract-ocr).
  - Printing needs a CUPS client (the lp command).
  - System speech needs espeak-ng or espeak.
  - API speech supports ElevenLabs or OpenRouter with your own credentials.
    Playback needs ffplay (FFmpeg) or mpg123; MP3 export needs no player.
    API keys use an unlocked Secret Service keyring (GNOME Keyring or KWallet).

LawPDF does not update itself on Linux. Download a newer release and extract it
into a new directory, then run its install.sh, or rebuild from source.

Licensed under the MIT License; see LICENSE and THIRD_PARTY_NOTICES.md.
README

# The packaged tree must satisfy the same runtime contract the other platforms
# are held to, verified from a working directory outside the tree so that only
# executable-relative probing is exercised.
VERIFY_WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/lawpdf-package-verify.XXXXXX")"
trap 'rm -rf "$VERIFY_WORKDIR"' EXIT
(
  cd "$VERIFY_WORKDIR"
  python3 - "$STAGE" <<'PYCODE'
import ctypes, json, os, pathlib, subprocess, sys
stage = pathlib.Path(sys.argv[1])
env = {key: value for key, value in os.environ.items()
       if key != "LAWPDF_MODEL_DIR" and not key.startswith("LAWPDF_LM2_")}
result = subprocess.run([str(stage / "lawpdf"), "--lm2-runtime-status",
                         "--require-native", "--require-context", "--require-arbiter",
                         "--require-note-head", "--require-link-ranker"],
                        env=env, capture_output=True, text=True, check=True)
report = json.loads(result.stdout)
if not report.get("requirements_met"):
    raise SystemExit("Packaged model runtime did not meet its requirements")
(stage / "runtime-verification.json").write_text(result.stdout, encoding="utf-8")
library = ctypes.CDLL(str(stage / "libpdfium.so"))
library.FPDF_InitLibrary()
library.FPDF_DestroyLibrary()
PYCODE
)

TARBALL="$DIST_DIR/LawPDF-linux-x86_64.tar.gz"
rm -f "$TARBALL"
tar -czf "$TARBALL" -C "$DIST_DIR" "$(basename "$STAGE")"

(cd "$DIST_DIR" && sha256sum "$(basename "$TARBALL")" > SHA256SUMS-linux.txt)
echo "$STAGE"
echo "$TARBALL"
