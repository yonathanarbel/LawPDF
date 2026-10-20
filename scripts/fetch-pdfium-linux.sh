#!/usr/bin/env bash
# Fetch the immutable, checksum-pinned PDFium Linux x86_64 release.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE="chromium/8057"
ARCHIVE_SHA256="7788fa57a2996ebd21afc4090eb0a42b9f95ef3a72e7ef4c0756f1ea703c0415"
LIBRARY_SHA256="08f6ea53a64f6fbb9f5ba8d9c02e0051987c6a9175f3fb5ba25f219174731aaa"
DESTINATION="$ROOT/vendor/libpdfium.so"

verify_sha256() {
  python3 - "$1" "$2" <<'PY'
import hashlib, pathlib, sys
path = pathlib.Path(sys.argv[1])
actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
raise SystemExit(0 if actual == sys.argv[2] else 1)
PY
}

if verify_sha256 "$DESTINATION" "$LIBRARY_SHA256"; then
  echo "Pinned PDFium Linux runtime already verified."
  exit 0
fi
WORK="$(mktemp -d "${TMPDIR:-/tmp}/lawpdf-pdfium.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
curl --fail --location --silent --show-error --retry 3 --connect-timeout 20 --max-time 300 \
  "https://github.com/bblanchon/pdfium-binaries/releases/download/$RELEASE/pdfium-linux-x64.tgz" \
  --output "$WORK/pdfium.tgz"
if ! verify_sha256 "$WORK/pdfium.tgz" "$ARCHIVE_SHA256"; then
  echo "error: PDFium archive checksum mismatch; no runtime was installed." >&2
  exit 1
fi
tar -xzf "$WORK/pdfium.tgz" -C "$WORK" lib/libpdfium.so
if ! verify_sha256 "$WORK/lib/libpdfium.so" "$LIBRARY_SHA256"; then
  echo "error: PDFium library checksum mismatch; no runtime was installed." >&2
  exit 1
fi
mkdir -p "$(dirname "$DESTINATION")"
install -m 0644 "$WORK/lib/libpdfium.so" "$DESTINATION"
echo "Verified PDFium $RELEASE Linux x86_64 runtime: $DESTINATION"
