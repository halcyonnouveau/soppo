#!/bin/sh
set -e

REPO="halcyonnouveau/sopmod"
INSTALL_DIR="${SOPMOD_INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS
case "$(uname -s)" in
    Linux*)  OS="unknown-linux-gnu" ;;
    Darwin*) OS="apple-darwin" ;;
    MINGW*|MSYS*|CYGWIN*) OS="pc-windows-msvc" ;;
    *) echo "Unsupported OS: $(uname -s)"; exit 1 ;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

TARGET="${ARCH}-${OS}"

# Get latest version
VERSION=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$VERSION" ]; then
    echo "Failed to get latest version"
    exit 1
fi

echo "Installing sopmod ${VERSION} for ${TARGET}..."

# Download
URL="https://github.com/${REPO}/releases/download/${VERSION}/sopmod-${TARGET}.tar.gz"
TEMP_DIR=$(mktemp -d)
trap "rm -rf ${TEMP_DIR}" EXIT

curl -sL "$URL" | tar xz -C "$TEMP_DIR"

# Install
mkdir -p "$INSTALL_DIR"
mv "${TEMP_DIR}/sopmod" "$INSTALL_DIR/sopmod"
chmod +x "$INSTALL_DIR/sopmod"

echo "Installed sopmod to ${INSTALL_DIR}/sopmod"

# Check if in PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add ${INSTALL_DIR} to your PATH to use sopmod" ;;
esac

echo ""
echo "Next steps:"
echo "  1. Run 'sopmod install sop latest' to install sop"
echo "  2. Run 'sopmod default sop latest' to set it as default"
echo "  3. Add ~/.sopmod/bin to your PATH:"
echo ""
echo "     export PATH=\"\$HOME/.sopmod/bin:\$PATH\""
