#!/bin/bash
set -euo pipefail

# Flatpak build and test script for Elysiae

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${PROJECT_ROOT}/build"
REPO_DIR="${PROJECT_ROOT}/repo"
FLATPAK_ID="app.elysiae.Elysiae"

FORCE_CLEAN=0
NO_CACHE=0
MANIFEST="${PROJECT_ROOT}/app.elysiae.Elysiae.yml"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --force-clean) FORCE_CLEAN=1; shift ;;
        --no-cache)    NO_CACHE=1;    shift ;;
        --local)       MANIFEST="${SCRIPT_DIR}/flatpak-local.yml"; shift ;;
        -h|--help)
            echo "Usage: $0 [--force-clean] [--no-cache] [--local]"
            echo ""
            echo "  --force-clean  Delete build dir before building"
            echo "  --no-cache     Disable flatpak-builder cache (full rebuild)"
            echo "  --local        Use local source manifest instead of remote"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

echo "=== Elysiae Flatpak Build Script ==="
echo "Project root: ${PROJECT_ROOT}"
echo "Manifest:     ${MANIFEST}"

if [[ ${FORCE_CLEAN} -eq 1 ]]; then
    echo "Cleaning up previous builds..."
    rm -rf "${BUILD_DIR}" "${REPO_DIR}"
fi
mkdir -p "${BUILD_DIR}" "${REPO_DIR}"

BUILDER_ARGS=()
if [[ ${FORCE_CLEAN} -eq 1 ]]; then BUILDER_ARGS+=(--force-clean); fi
if [[ ${NO_CACHE} -eq 1 ]];    then BUILDER_ARGS+=(--disable-cache); fi

echo "Building flatpak..."
flatpak run org.flatpak.Builder \
    "${BUILDER_ARGS[@]}" \
    --repo="${REPO_DIR}" \
    "${BUILD_DIR}" \
    "${MANIFEST}"

# Create a test flatpak repo
echo "Creating flatpak repository..."
flatpak build-bundle "${REPO_DIR}" "${BUILD_DIR}/${FLATPAK_ID}.flatpak" "${FLATPAK_ID}"

echo "=== Build Complete ==="
echo "Flatpak bundle: ${BUILD_DIR}/${FLATPAK_ID}.flatpak"
echo ""
echo "To install: flatpak install --user ${BUILD_DIR}/${FLATPAK_ID}.flatpak"
echo "To run: flatpak run ${FLATPAK_ID}"
