#!/bin/sh
set -eu
version=${1:?Supply the release tag}
image=${2:?Supply the fully qualified server image name}
out=${3:-artifacts}
case "$version" in
    v*[!a-zA-Z0-9._-]*|v|'') echo "Expected an OCI-compatible v-prefixed release tag" >&2; exit 1 ;;
    v*) ;;
    *) echo "Expected a v-prefixed release tag" >&2; exit 1 ;;
esac
case "$image" in
    *[!a-z0-9./_:-]*|'') echo "Invalid image name" >&2; exit 1 ;;
esac
mkdir -p "$out"
out=$(cd "$out" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
bundle=$scratch/rally
mkdir -p "$bundle/scripts" "$bundle/examples"
sed "s|^        image: ghcr.io/arctic-hen7/rally:.*|        image: $image:$version|" examples/compose.yml > "$bundle/compose.yml"
cp scripts/setup-compose.sh "$bundle/scripts/"
cp examples/config.toml "$bundle/examples/"
cp docs/installation.md "$bundle/INSTALL.md"
cp LICENSE-* "$bundle/"
archive=rally-compose.tar.gz
tar -czf "$out/$archive" -C "$scratch" rally
(cd "$out" && sha256sum "$archive" > "$archive.sha256")
