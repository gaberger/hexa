#!/usr/bin/env bash
# Install hexa from the latest GitHub release, or a given version.
#
#   curl -fsSL https://raw.githubusercontent.com/gaberger/hexa/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/gaberger/hexa/main/install.sh | bash -s -- 26.9.0
#
# Puts one binary, `hexa`, in $HEXA_INSTALL_DIR (default ~/.local/bin) and
# verifies it against the release's SHA256SUMS.txt. After that, `hexa
# self-update` keeps it current.
set -euo pipefail

REPO="gaberger/hexa"
DIR="${HEXA_INSTALL_DIR:-$HOME/.local/bin}"
WANT="${1:-}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)            TARGET=aarch64-apple-darwin ;;
  Darwin-x86_64)           TARGET=x86_64-apple-darwin ;;
  Linux-x86_64)            TARGET=x86_64-unknown-linux-gnu ;;
  Linux-aarch64|Linux-arm64) TARGET=aarch64-unknown-linux-gnu ;;
  *) echo "no release for $(uname -s)/$(uname -m); build from source: cargo build -p hexa-cli --release" >&2; exit 1 ;;
esac

if [[ -z "$WANT" ]]; then
  TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [[ -n "$TAG" ]] || { echo "could not read the latest release tag" >&2; exit 1; }
else
  TAG="v${WANT#v}"
fi
VERSION="${TAG#v}"
TARBALL="hexa-${VERSION}-${TARGET}.tar.gz"
BASE="https://github.com/${REPO}/releases/download/${TAG}"

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
echo "hexa ${VERSION} for ${TARGET}"
curl -fsSL -o "$TMP/$TARBALL" "$BASE/$TARBALL"
curl -fsSL -o "$TMP/SHA256SUMS.txt" "$BASE/SHA256SUMS.txt"
( cd "$TMP" && grep " ${TARBALL}\$" SHA256SUMS.txt | sha256sum -c - >/dev/null ) || { echo "checksum mismatch for $TARBALL" >&2; exit 1; }
tar xzf "$TMP/$TARBALL" -C "$TMP"
mkdir -p "$DIR"
install -m 755 "$TMP/hexa" "$DIR/hexa"
echo "installed $("$DIR/hexa" --version) to $DIR/hexa"
case ":$PATH:" in *":$DIR:"*) ;; *) echo "note: $DIR is not on your PATH" ;; esac
