#!/bin/sh
set -eu
version=${1:?Supply the release tag}
platform=${2:?Supply the platform name}
out=${3:-artifacts}
bin_dir="${CARGO_TARGET_DIR:-target}/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"
mkdir -p "$out"
out=$(cd "$out" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
name="rally-$version-$platform"
cli="rally-cli-$version-$platform"
mkdir -p "$scratch/$name/bin"
cp "$bin_dir/rally" "$out/$cli"
cp "$out/$cli" "$scratch/$name/bin/rally"
cp LICENSE-MIT LICENSE-APACHE README.md "$scratch/$name/"
if [ "${platform#linux-}" != "$platform" ]; then
    cp "$bin_dir/rally-server" "$scratch/$name/bin/"
    cp -R docs examples runtime scripts "$scratch/$name/"
    mkdir -p "$scratch/$name/web"
    cp -R web/dist "$scratch/$name/web/"
fi
tar -czf "$out/$name.tar.gz" -C "$scratch" "$name"
cd "$out"
for asset in "$cli" "$name.tar.gz"; do
    if command -v sha256sum >/dev/null; then
        sha256sum "$asset" >"$asset.sha256"
    else
        shasum -a 256 "$asset" >"$asset.sha256"
    fi
done
