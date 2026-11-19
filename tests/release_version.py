"""Verify that release artifacts use the pushed tag even when Cargo starts at another version."""

import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as tmp:
    checkout = Path(tmp)
    for file in ("Cargo.toml", "Cargo.lock", "LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy(root / file, checkout)
    for directory in ("src", "docs"):
        shutil.copytree(root / directory, checkout / directory)
    original_lock = tomllib.loads((checkout / "Cargo.lock").read_text())["package"]
    for version in ("0.1.0-rc.2", "1.2.3-RC.7", "1.2.3"):
        subprocess.run(
            ["python3", root / "scripts/release-version.py"],
            cwd=checkout, env=dict(os.environ, GITHUB_REF=f"refs/tags/v{version}"), check=True,
        )
        packages = tomllib.loads((checkout / "Cargo.lock").read_text())["package"]
        assert {p["name"]: p["version"] for p in packages if "source" not in p} == {
            "rally-cli": version, "rally-server": version,
        }
        assert [p for p in packages if "source" in p] == [p for p in original_lock if "source" in p]
        subprocess.run(
            ["cargo", "package", "--locked", "--no-verify", "-p", "rally-cli", "--target-dir", "target"],
            cwd=checkout, check=True,
        )
        with tarfile.open(checkout / f"target/package/rally-cli-{version}.crate") as archive:
            manifest = archive.extractfile(f"rally-cli-{version}/Cargo.toml")
            assert tomllib.load(manifest)["package"]["version"] == version
    print("Verified the release tag supplies both workspace versions and the crates.io package version")
