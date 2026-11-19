"""Apply the pushed Git tag to the Cargo workspace in the release checkout."""

import os
from pathlib import Path
import re
import subprocess


def main():
    ref = os.environ.get("GITHUB_REF", "")
    match = re.fullmatch(r"refs/tags/v([0-9][0-9A-Za-z.-]*)", ref)
    if not match or len(match[1]) + 1 > 128:
        raise SystemExit("Expected a v-prefixed Cargo version usable as a container tag")

    version = match[1]
    manifest = Path("Cargo.toml")
    lines = manifest.read_text().splitlines(keepends=True)
    in_workspace_package = False
    for i, line in enumerate(lines):
        if line.startswith("["):
            in_workspace_package = line.strip() == "[workspace.package]"
        elif in_workspace_package and re.match(r"^version\s*=", line):
            lines[i] = f'version = "{version}"\n'
            break
    else:
        raise SystemExit("Missing workspace.package.version in Cargo.toml")
    manifest.write_text("".join(lines))
    # Cargo validates the version and updates workspace entries in the lockfile
    subprocess.run(["cargo", "update", "--workspace"], check=True)


if __name__ == "__main__":
    main()
