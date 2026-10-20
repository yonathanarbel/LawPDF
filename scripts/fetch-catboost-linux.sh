#!/usr/bin/env bash
# Fetch the checksum-pinned official CatBoost Linux x86_64 model runtime.
set -euo pipefail
VERSION="${1:-1.2.10}"
EXPECTED_SHA256="de32f8e147ee8f969599a00b7145e8561b943e916b0132b811037f8f71952a0e"
if [[ "$VERSION" != "1.2.10" ]]; then
  echo "error: No pinned CatBoost checksum is registered for $VERSION." >&2
  exit 1
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIBRARY_NAME="libcatboostmodel-linux-x86_64-$VERSION.so"
DESTINATION="$ROOT/profile-models/lm2-native-catboost-runtime/$LIBRARY_NAME"
verify_sha256() {
  python3 - "$1" "$EXPECTED_SHA256" <<'PY'
import hashlib, pathlib, sys
path = pathlib.Path(sys.argv[1])
actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
raise SystemExit(0 if actual == sys.argv[2] else 1)
PY
}
if verify_sha256 "$DESTINATION"; then
  echo "Pinned CatBoost Linux runtime already verified."
  exit 0
fi
WORK="$(mktemp -d "${TMPDIR:-/tmp}/lawpdf-catboost.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
curl --fail --location --silent --show-error --retry 3 --connect-timeout 20 --max-time 300 \
  "https://github.com/catboost/catboost/releases/download/v$VERSION/$LIBRARY_NAME" \
  --output "$WORK/$LIBRARY_NAME"
if ! verify_sha256 "$WORK/$LIBRARY_NAME"; then
  echo "error: CatBoost library checksum mismatch; no runtime was installed." >&2
  exit 1
fi
mkdir -p "$(dirname "$DESTINATION")"
install -m 0644 "$WORK/$LIBRARY_NAME" "$DESTINATION"
echo "Verified CatBoost $VERSION Linux x86_64 runtime: $DESTINATION"
