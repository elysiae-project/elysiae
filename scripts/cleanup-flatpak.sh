#!/bin/bash
set -euo pipefail

# Cleanup script for flatpak builds
# This script removes build artifacts and cached flatpak data to save disk space

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "=== Elysiae Flatpak Cleanup Script ==="

# Remove build directories
echo "Removing build directories..."
rm -rf "${PROJECT_ROOT}/build"
rm -rf "${PROJECT_ROOT}/repo"
rm -rf "${PROJECT_ROOT}/.flatpak-builder"

# Remove flatpak builder cache (optional, but saves a lot of space)
if [ -d "${HOME}/.local/share/flatpak-builder" ]; then
    echo "Removing flatpak-builder cache..."
    rm -rf "${HOME}/.local/share/flatpak-builder"
fi

# Remove any leftover flatpak bundles
find "${PROJECT_ROOT}" -name "*.flatpak" -type f -delete

echo "=== Cleanup Complete ==="
