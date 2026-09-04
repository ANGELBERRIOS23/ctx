#!/usr/bin/env bash
# ==============================================================================
# ctx CLI Client Installer
#
# Detects operating system and machine architecture, downloads the matching
# ctx binary from GitHub Releases (placeholder URL), installs it to ~/.local/bin,
# and configures the user's PATH environment variable if necessary.
# ==============================================================================

set -euo pipefail

# ANSI color codes
BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

INSTALL_DIR="${HOME}/.local/bin"
BINARY_NAME="ctx"
GITHUB_REPO="${CTX_REPO:-ctx-sync/ctx}"
VERSION="${CTX_VERSION:-latest}"

echo -e "${BOLD}${BLUE}"
echo " ctx CLI Installer"
echo -e "──────────────────────────────────────────${NC}"

# 1. Detect Operating System
OS_TYPE="$(uname -s)"
case "${OS_TYPE}" in
    Linux*)
        OS="linux"
        ;;
    Darwin*)
        OS="darwin"
        ;;
    CYGWIN*|MINGW*|MSYS*)
        OS="windows"
        BINARY_NAME="ctx.exe"
        ;;
    *)
        echo -e "${RED}Error: Unsupported operating system: ${OS_TYPE}${NC}" >&2
        exit 1
        ;;
esac

# 2. Detect Machine Architecture
ARCH_TYPE="$(uname -m)"
case "${ARCH_TYPE}" in
    x86_64|amd64)
        ARCH="x86_64"
        ;;
    arm64|aarch64)
        ARCH="aarch64"
        ;;
    armv7l|armhf)
        ARCH="armv7"
        ;;
    *)
        echo -e "${RED}Error: Unsupported CPU architecture: ${ARCH_TYPE}${NC}" >&2
        exit 1
        ;;
esac

echo -e "Detected platform: ${BOLD}${OS}-${ARCH}${NC}"

# Construct artifact and target URL
# Release archive naming convention: ctx-<os>-<arch>.tar.gz
RELEASE_ARCHIVE="ctx-${OS}-${ARCH}.tar.gz"
if [ "${VERSION}" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${RELEASE_ARCHIVE}"
else
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${RELEASE_ARCHIVE}"
fi

# Fallback direct binary URL (if archive is not used)
DIRECT_BINARY_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/ctx-${OS}-${ARCH}"

# 3. Create install directory
mkdir -p "${INSTALL_DIR}"

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

echo -e "Downloading ctx CLI from: ${BLUE}${DOWNLOAD_URL}${NC}"

DOWNLOAD_SUCCESS=false

# Helper function to download file
fetch_url() {
    local url="$1"
    local output="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "${url}" -o "${output}"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "${output}" "${url}"
    else
        echo -e "${RED}Error: curl or wget is required to download ctx.${NC}" >&2
        exit 1
    fi
}

# Attempt download of tar.gz archive, then direct binary
if fetch_url "${DOWNLOAD_URL}" "${TMP_DIR}/${RELEASE_ARCHIVE}" 2>/dev/null; then
    echo -e "Extracting archive..."
    tar -xzf "${TMP_DIR}/${RELEASE_ARCHIVE}" -C "${TMP_DIR}"
    if [ -f "${TMP_DIR}/${BINARY_NAME}" ]; then
        mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
        DOWNLOAD_SUCCESS=true
    elif [ -f "${TMP_DIR}/ctx" ]; then
        mv "${TMP_DIR}/ctx" "${INSTALL_DIR}/${BINARY_NAME}"
        DOWNLOAD_SUCCESS=true
    fi
elif fetch_url "${DIRECT_BINARY_URL}" "${TMP_DIR}/${BINARY_NAME}" 2>/dev/null; then
    mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    DOWNLOAD_SUCCESS=true
fi

# Check if placeholder download failed and check for local release build
if [ "${DOWNLOAD_SUCCESS}" = false ]; then
    echo -e "${YELLOW}Warning: Remote release not found at GitHub releases URL (placeholder repository).${NC}"
    
    # Check if a locally compiled ctx binary is available in the workspace
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    LOCAL_RELEASE="${SCRIPT_DIR}/../target/release/ctx"
    LOCAL_DEBUG="${SCRIPT_DIR}/../target/debug/ctx"
    
    if [ -f "${LOCAL_RELEASE}" ]; then
        echo -e "Found local release binary at ${LOCAL_RELEASE}. Installing..."
        cp "${LOCAL_RELEASE}" "${INSTALL_DIR}/${BINARY_NAME}"
        DOWNLOAD_SUCCESS=true
    elif [ -f "${LOCAL_DEBUG}" ]; then
        echo -e "Found local debug binary at ${LOCAL_DEBUG}. Installing..."
        cp "${LOCAL_DEBUG}" "${INSTALL_DIR}/${BINARY_NAME}"
        DOWNLOAD_SUCCESS=true
    else
        echo -e "${RED}To build from source, run:${NC}"
        echo -e "  cargo build --release --bin ctx"
        echo -e "  cp target/release/ctx ${INSTALL_DIR}/${BINARY_NAME}"
        exit 1
    fi
fi

# Make binary executable
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
echo -e "${GREEN}✓ Installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}${NC}"

# 4. PATH Detection and Configuration
CURRENT_SHELL="$(basename "${SHELL:-bash}")"
PATH_ALREADY_SET=false

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        PATH_ALREADY_SET=true
        ;;
esac

if [ "${PATH_ALREADY_SET}" = false ]; then
    echo -e "${YELLOW}${INSTALL_DIR} is not in your current PATH.${NC}"

    PROFILE_FILE=""
    if [ "${CURRENT_SHELL}" = "zsh" ]; then
        PROFILE_FILE="${HOME}/.zshrc"
    elif [ "${CURRENT_SHELL}" = "bash" ]; then
        if [ -f "${HOME}/.bash_profile" ]; then
            PROFILE_FILE="${HOME}/.bash_profile"
        else
            PROFILE_FILE="${HOME}/.bashrc"
        fi
    else
        PROFILE_FILE="${HOME}/.profile"
    fi

    EXPORT_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""

    if [ -n "${PROFILE_FILE}" ]; then
        if ! grep -q "${INSTALL_DIR}" "${PROFILE_FILE}" 2>/dev/null; then
            echo "" >> "${PROFILE_FILE}"
            echo "# Added by ctx CLI installer" >> "${PROFILE_FILE}"
            echo "${EXPORT_LINE}" >> "${PROFILE_FILE}"
            echo -e "${GREEN}✓ Added ${INSTALL_DIR} to ${PROFILE_FILE}${NC}"
            echo -e "  To apply in the current terminal, run: ${BOLD}source ${PROFILE_FILE}${NC}"
        fi
    fi
fi

# 5. Verification & Summary
echo ""
echo -e "${BOLD}${GREEN}Installation Complete!${NC}"
echo -e "Binary location: ${BLUE}${INSTALL_DIR}/${BINARY_NAME}${NC}"
echo ""
echo -e "${BOLD}Get Started:${NC}"
echo -e "  1. Connect to your ctx server:"
echo -e "     ${YELLOW}ctx connect http://<server-ip>:9900${NC}"
echo ""
echo -e "  2. Log in or create an account:"
echo -e "     ${YELLOW}ctx login${NC}"
echo ""
echo -e "  3. Initialize current project directory:"
echo -e "     ${YELLOW}ctx init${NC}"
echo ""
echo -e "  4. Check status & sync:"
echo -e "     ${YELLOW}ctx status${NC}"
echo -e "     ${YELLOW}ctx push${NC}"
echo ""
