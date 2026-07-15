#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="LawPDF"
PROFILE="${LAWPDF_MACOS_PROFILE:-release}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BUILD_ARGS=(--locked)
VERSION="$(awk -F ' *= *' '$1 == "version" {gsub(/\"/, "", $2); print $2; exit}' "$ROOT/Cargo.toml")"

if [[ "$PROFILE" == "release" ]]; then
  BUILD_ARGS+=(--release)
fi

cd "$ROOT"

cargo build "${BUILD_ARGS[@]}"

BIN="$TARGET_DIR/$PROFILE/lawpdf"
APP_DIR="$ROOT/dist/$APP_NAME.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
FRAMEWORKS="$CONTENTS/Frameworks"

rm -rf "$APP_DIR"
mkdir -p "$MACOS" "$RESOURCES" "$FRAMEWORKS"

cp "$BIN" "$MACOS/$APP_NAME"
chmod +x "$MACOS/$APP_NAME"

if [[ -f "$ROOT/vendor/libpdfium.dylib" ]]; then
  cp "$ROOT/vendor/libpdfium.dylib" "$FRAMEWORKS/libpdfium.dylib"
elif [[ -n "${PDFIUM_DYNAMIC_LIB_PATH:-}" && -f "$PDFIUM_DYNAMIC_LIB_PATH" ]]; then
  cp "$PDFIUM_DYNAMIC_LIB_PATH" "$FRAMEWORKS/libpdfium.dylib"
else
  echo "warning: libpdfium.dylib was not bundled; set PDFIUM_DYNAMIC_LIB_PATH or place it at vendor/libpdfium.dylib" >&2
fi

cp "$ROOT/assets/lawpdf.png" "$RESOURCES/lawpdf.png"

if [[ "${LAWPDF_BUNDLE_PP_DOCLAYOUT:-0}" == "1" ]]; then
  mkdir -p "$RESOURCES/tools"
  if [[ -f "$ROOT/tools/lm2_pp_doclayout_regions.py" ]]; then
    cp "$ROOT/tools/lm2_pp_doclayout_regions.py" "$RESOURCES/tools/lm2_pp_doclayout_regions.py"
  fi

  if [[ -d "$ROOT/.lawpdf/ppdoclayout-venv" ]]; then
    rm -rf "$RESOURCES/ppdoclayout-venv"
    cp -R "$ROOT/.lawpdf/ppdoclayout-venv" "$RESOURCES/ppdoclayout-venv"
    find "$RESOURCES/ppdoclayout-venv" \( -iname "fitz" -o -iname "pymupdf" -o -iname "pymupdf-*.dist-info" -o -iname "pymupdf" \) -prune -exec rm -rf {} +
    rm -f "$RESOURCES/ppdoclayout-venv/bin/pymupdf"
  else
    echo "warning: PP-DocLayout venv was not bundled; expected $ROOT/.lawpdf/ppdoclayout-venv" >&2
  fi
else
  echo "PP-DocLayout sidecar assets not bundled; set LAWPDF_BUNDLE_PP_DOCLAYOUT=1 to include them" >&2
fi

if [[ -d "$ROOT/profile-models/lm2-current" ]]; then
  mkdir -p "$RESOURCES/profile-models"
  rm -rf "$RESOURCES/profile-models/lm2-current"
  cp -R "$ROOT/profile-models/lm2-current" "$RESOURCES/profile-models/lm2-current"
fi

if [[ -d "$ROOT/profile-models/lm2-v20-runtime" ]]; then
  mkdir -p "$RESOURCES/profile-models"
  rm -rf "$RESOURCES/profile-models/lm2-v20-runtime"
  cp -R "$ROOT/profile-models/lm2-v20-runtime" "$RESOURCES/profile-models/lm2-v20-runtime"
fi

NATIVE_RUNTIME_SOURCE="$ROOT/profile-models/lm2-native-catboost-runtime"
CONTEXT_RUNTIME_SOURCE="$ROOT/profile-models/lm2-context-twopass-runtime"
NATIVE_MODEL="lm2-catboost-augmented-epoch51lv-relabels-tc.cbm"
NATIVE_LIBRARY="libcatboostmodel-darwin-universal2-1.2.10.dylib"
CONTEXT_MODEL="lm2-context-twopass-hgb-v1.json"
for asset in \
  "$NATIVE_RUNTIME_SOURCE/$NATIVE_MODEL" \
  "$NATIVE_RUNTIME_SOURCE/$NATIVE_LIBRARY" \
  "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_MODEL"; do
  if [[ ! -f "$asset" ]]; then
    echo "missing promoted LM2 runtime asset: $asset" >&2
    exit 1
  fi
done

NATIVE_RUNTIME_DEST="$RESOURCES/profile-models/lm2-native-catboost-runtime"
CONTEXT_RUNTIME_DEST="$RESOURCES/profile-models/lm2-context-twopass-runtime"
mkdir -p "$NATIVE_RUNTIME_DEST" "$CONTEXT_RUNTIME_DEST"
cp "$NATIVE_RUNTIME_SOURCE/$NATIVE_MODEL" "$NATIVE_RUNTIME_DEST/$NATIVE_MODEL"
cp "$NATIVE_RUNTIME_SOURCE/$NATIVE_LIBRARY" "$NATIVE_RUNTIME_DEST/$NATIVE_LIBRARY"
cp "$CONTEXT_RUNTIME_SOURCE/$CONTEXT_MODEL" "$CONTEXT_RUNTIME_DEST/$CONTEXT_MODEL"
cp "$ROOT/release-manifest.json" "$RESOURCES/release-manifest.json"

if command -v lipo >/dev/null 2>&1; then
  BIN_ARCHS="$(lipo -archs "$MACOS/$APP_NAME")"
  for library in \
    "$FRAMEWORKS/libpdfium.dylib" \
    "$NATIVE_RUNTIME_DEST/$NATIVE_LIBRARY"; do
    [[ -f "$library" ]] || continue
    LIB_ARCHS="$(lipo -archs "$library")"
    for arch in $BIN_ARCHS; do
      if [[ " $LIB_ARCHS " != *" $arch "* ]]; then
        echo "packaged library architecture mismatch: $library has [$LIB_ARCHS], executable needs [$BIN_ARCHS]" >&2
        exit 1
      fi
    done
  done
fi

cat > "$CONTENTS/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>LawPDF</string>
  <key>CFBundleExecutable</key>
  <string>LawPDF</string>
  <key>CFBundleIdentifier</key>
  <string>design.yarbel.lawpdf</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>LawPDF</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>__VERSION__</string>
  <key>CFBundleVersion</key>
  <string>__VERSION__</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
PLIST
sed -i.bak "s/__VERSION__/$VERSION/g" "$CONTENTS/Info.plist"
rm -f "$CONTENTS/Info.plist.bak"

VERIFY_WORKDIR="${TMPDIR:-/tmp}/lawpdf-package-runtime-verify"
mkdir -p "$VERIFY_WORKDIR"
(
  cd "$VERIFY_WORKDIR"
  "$MACOS/$APP_NAME" --lm2-runtime-status --require-native --require-context >/dev/null
)

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_DIR" >/dev/null 2>&1 || true
fi

ZIP="$ROOT/dist/LawPDF-macos.zip"
rm -f "$ZIP"
(cd "$ROOT/dist" && zip -r -q "$(basename "$ZIP")" "$APP_NAME.app")

echo "$APP_DIR"
echo "$ZIP"
