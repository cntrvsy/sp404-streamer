#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="${ROOT_DIR}/target/AppDir"
OUTPUT_DIR="${ROOT_DIR}/target/release-appimage"

echo "==> Building sp404-streamer release binary..."
cargo build --release --manifest-path "${ROOT_DIR}/Cargo.toml"

echo "==> Preparing AppDir layout..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin"
mkdir -p "${APP_DIR}/usr/share/icons/hicolor/128x128/apps"
mkdir -p "${APP_DIR}/usr/share/applications"

# Copy binary
cp "${ROOT_DIR}/target/release/sp404-streamer" "${APP_DIR}/usr/bin/"

# Copy desktop file and icons
cp "${ROOT_DIR}/assets/sp404-streamer.desktop" "${APP_DIR}/"
cp "${ROOT_DIR}/assets/sp404-streamer.desktop" "${APP_DIR}/usr/share/applications/"
cp "${ROOT_DIR}/assets/sp404-streamer.png" "${APP_DIR}/"
cp "${ROOT_DIR}/assets/sp404-streamer.png" "${APP_DIR}/usr/share/icons/hicolor/128x128/apps/"
cp "${ROOT_DIR}/assets/sp404-streamer.svg" "${APP_DIR}/"

# Copy AppRun
cp "${ROOT_DIR}/packaging/AppRun" "${APP_DIR}/AppRun"
chmod +x "${APP_DIR}/AppRun"

mkdir -p "${OUTPUT_DIR}"

# Check for appimagetool
if ! command -v appimagetool &> /dev/null; then
    echo "==> Downloading appimagetool..."
    TOOL_PATH="${ROOT_DIR}/target/appimagetool"
    if [ ! -f "${TOOL_PATH}" ]; then
        curl -sLo "${TOOL_PATH}" "https://github.com/AppImage/AppImageKit/releases/download/13/appimagetool-x86_64.AppImage"
        chmod +x "${TOOL_PATH}"
    fi
    APPIMAGETOOL="${TOOL_PATH}"
else
    APPIMAGETOOL="appimagetool"
fi

echo "==> Packaging AppImage..."
ARCH=x86_64 "${APPIMAGETOOL}" --appimage-extract-and-run "${APP_DIR}" "${OUTPUT_DIR}/sp404-streamer-x86_64.AppImage"

echo "==> Done! AppImage created at: ${OUTPUT_DIR}/sp404-streamer-x86_64.AppImage"
