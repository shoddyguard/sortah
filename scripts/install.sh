#!/usr/bin/env bash
# Installs the latest sortah release from GitHub.
# Run as root to install to /usr/bin (Linux) or /usr/local/bin (macOS),
# or as a regular user to install to ~/.local/bin.
set -euo pipefail

OWNER='shoddyguard'
REPO='sortah'
BINARY='sortah'

VERSION=$(curl -fsSL "https://api.github.com/repos/$OWNER/$REPO/releases/latest" \
    | grep '"tag_name"' \
    | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$VERSION" ]; then
    echo "Error: could not determine the latest release version." >&2
    exit 1
fi

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Linux)
        case "$ARCH" in
            x86_64) TARGET='x86_64-unknown-linux-gnu' ;;
            *)      echo "Error: unsupported architecture '$ARCH' on Linux." >&2; exit 1 ;;
        esac
        ;;
    Darwin)
        case "$ARCH" in
            arm64)  TARGET='aarch64-apple-darwin' ;;
            *)      echo "Error: unsupported architecture '$ARCH' on macOS." >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "Error: unsupported operating system '$OS'." >&2
        exit 1
        ;;
esac

ARCHIVE="${BINARY}-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/$OWNER/$REPO/releases/download/$VERSION/$ARCHIVE"

echo "Downloading $BINARY $VERSION for $TARGET..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "$TMPDIR/$ARCHIVE"
tar -xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"

if [ "$(id -u)" -eq 0 ]; then
    case "$OS" in
        Linux)  INSTALL_DIR='/usr/bin' ;;
        Darwin) INSTALL_DIR='/usr/local/bin' ;;
    esac
else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

echo "Installing $BINARY to $INSTALL_DIR..."
install -m 755 "$TMPDIR/$BINARY" "$INSTALL_DIR/$BINARY"

if [ "$(id -u)" -ne 0 ]; then
    case ":${PATH}:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            echo ""
            echo "NOTE: $INSTALL_DIR is not in your PATH."
            echo "Add the following to your shell profile to make $BINARY available:"
            echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
            ;;
    esac
fi

echo ""
echo "$BINARY $VERSION installed successfully."
