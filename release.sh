#!/bin/bash
# Cuts a new release.

set -euo pipefail

tag="${1:?Provide a release tag}"
if [[ "$tag" != "v"* ]]; then
    echo "Error: provide a tag beginning with 'v' for a new release."
    exit 1
fi

echo "Tagging new release '$tag'..."
git tag -a "$tag" -m "$tag"

echo "Pushing new release '$tag'..."
git push origin "$tag"
