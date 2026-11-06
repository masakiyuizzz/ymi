#!/bin/sh
# Build ymi and install it somewhere on PATH.
#   ./install.sh              -> ~/.local/bin
#   YMI_INSTALL_DIR=/usr/local/bin ./install.sh
set -eu

cd "$(dirname "$0")"
DEST="${YMI_INSTALL_DIR:-$HOME/.local/bin}"

cargo build --release

mkdir -p "$DEST"
# Write to a fresh inode and rename over the old one: overwriting in place
# keeps com.apple.provenance on the file and macOS then kills it (exit 137).
cat target/release/ymi > "$DEST/.ymi.new"
chmod +x "$DEST/.ymi.new"
mv -f "$DEST/.ymi.new" "$DEST/ymi"

if command -v xattr >/dev/null 2>&1; then
    xattr -c "$DEST/ymi" 2>/dev/null || true
fi

printf 'installed %s -> %s\n' "$("$DEST/ymi" -v)" "$DEST/ymi"

case ":$PATH:" in
    *":$DEST:"*) ;;
    *) printf 'note: %s is not on PATH, add it to your shell profile\n' "$DEST" ;;
esac
