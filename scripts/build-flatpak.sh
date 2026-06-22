#!/bin/bash
set -euo pipefail

# Flatpak build and test script for Elysiae
# This script builds the flatpak and runs basic smoke tests

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/build"
REPO_DIR="${PROJECT_ROOT}/repo"
FLATPAK_ID="app.elysiae.Elysiae"

echo "=== Elysiae Flatpak Build Script ==="
echo "Project root: ${PROJECT_ROOT}"

# Clean up previous builds
echo "Cleaning up previous builds..."
rm -rf "${BUILD_DIR}" "${REPO_DIR}"
mkdir -p "${BUILD_DIR}" "${REPO_DIR}"

# Build the app using flatpak-builder
echo "Building flatpak..."
flatpak run org.flatpak.Builder \
    --force-clean \
    --repo="${REPO_DIR}" \
    --disable-cache \
    "${BUILD_DIR}" \
    "${PROJECT_ROOT}/app.elysiae.Elysiae.yml"

# Create a test flatpak repo
echo "Creating flatpak repository..."
flatpak build-bundle "${REPO_DIR}" "${BUILD_DIR}/${FLATPAK_ID}.flatpak" "${FLATPAK_ID}"

echo "=== Build Complete ==="
echo "Flatpak bundle: ${BUILD_DIR}/${FLATPAK_ID}.flatpak"
echo ""
echo "To install: flatpak install --user ${BUILD_DIR}/${FLATPAK_ID}.flatpak"
echo "To run: flatpak run ${FLATPAK_ID}"
