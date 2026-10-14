# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# Adapted from SolidCraft's and VectorCraft's packaging (MIT OR Apache-2.0; see ATTRIBUTION.md).
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 [workspace.package] version from Cargo.toml (override: FIELDCRAFT_VERSION)
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   FIELDCRAFT_BUILD_SHA    the commit being packaged (recorded in the macOS Info.plist)
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)
#   APP_ID                  the reverse-DNS id: macOS bundle id, Linux .desktop and icon name, and
#                           the window's Wayland app_id / X11 class (apps/fieldcraft/src/main.rs)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${FIELDCRAFT_VERSION:-$(workspace_version)}"
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
DIST="$(cd "$DIST" && pwd)"
export DIST

FIELDCRAFT_BUILD_SHA="${FIELDCRAFT_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)}"
export FIELDCRAFT_BUILD_SHA
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
APP_ID=io.github.jyviko.fieldcraft
export APP_ID

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# The readme, licence, notice and third-party licence files every package carries
# (packaging/windows/package.ps1 has the same list).
DOC_FILES=(README.md LICENSE-MIT LICENSE-APACHE NOTICE THIRD-PARTY-LICENSES.txt)

# Copy DOC_FILES into a package directory.
copy_docs() {
  local dest="$1" f
  for f in "${DOC_FILES[@]}"; do
    cp "$ROOT/$f" "$dest/"
  done
}
