#!/usr/bin/env bash
# macOS/Linux equivalent of build.ps1: keep every generated tree outside Box.
set -euo pipefail
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$PROJECT_DIR")"
EXTERNAL_ROOT="${LAWPDF_ANDROID_EXTERNAL_ROOT:-${TMPDIR:-/tmp}/lawpdf-android}"
case "$EXTERNAL_ROOT" in
  "$REPO_DIR"|"$REPO_DIR"/*) echo "Android build output must be outside this checkout." >&2; exit 1 ;;
  /*) ;;
  *) echo "LAWPDF_ANDROID_EXTERNAL_ROOT must be an absolute path." >&2; exit 1 ;;
esac
: "${ANDROID_HOME:?Set ANDROID_HOME to an Android SDK containing android-35}"
export LAWPDF_ANDROID_BUILD_DIR="$EXTERNAL_ROOT/build"
export GRADLE_USER_HOME="$EXTERNAL_ROOT/gradle-user-home"
mkdir -p "$EXTERNAL_ROOT/project-cache" "$GRADLE_USER_HOME"
if [[ "$#" == 0 ]]; then set -- assembleDebug; fi
exec sh "$PROJECT_DIR/gradlew" -p "$PROJECT_DIR" --no-daemon \
  --project-cache-dir "$EXTERNAL_ROOT/project-cache" --console plain "$@"
