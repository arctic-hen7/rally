#!/bin/sh
set -eu
version=${1:?Supply the release tag}
platform=${2:?Supply the platform name}
out=${3:-artifacts}
mkdir -p "$out"
out=$(cd "$out" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
name="rally-$version-$platform"
mkdir -p "$scratch/$name/bin"
cp target/release/rally "$scratch/$name/bin/"
cp LICENSE-MIT LICENSE-APACHE README.md "$scratch/$name/"
if [ "${platform#linux-}" != "$platform" ]; then
    cp target/release/rally-server "$scratch/$name/bin/"
    cp -R docs examples runtime scripts "$scratch/$name/"
    mkdir -p "$scratch/$name/web"
    cp -R web/dist "$scratch/$name/web/"
fi
tar -czf "$out/$name.tar.gz" -C "$scratch" "$name"
cd "$out"
if command -v sha256sum >/dev/null; then
    sha256sum "$name.tar.gz" >"$name.tar.gz.sha256"
else
    shasum -a 256 "$name.tar.gz" >"$name.tar.gz.sha256"
fi
