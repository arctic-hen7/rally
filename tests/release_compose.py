"""Check the installation bundle in an empty directory with the Compose resolver."""

import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

root = Path(__file__).resolve().parents[1]
release = "v1.2.3-test.1"
image = "registry.example.test:5443/example/rally"
env = {k: v for k, v in os.environ.items() if not k.startswith(("RALLY_", "COMPOSE_"))}
env.pop("DATABASE_URL", None)

with tempfile.TemporaryDirectory() as tmp:
    archive = Path(tmp) / "rally-compose.tar.gz"
    subprocess.run(
        ["sh", "scripts/package-compose.sh", release, image, tmp], cwd=root, check=True
    )
    subprocess.run(["sha256sum", "-c", archive.name + ".sha256"], cwd=tmp, check=True)
    with tarfile.open(archive) as bundle:
        bundle.extractall(tmp, filter="data")
    install = Path(tmp) / "rally"
    subprocess.run(["sh", "scripts/setup-compose.sh"], cwd=install, env=env, check=True)
    with (install / ".env").open("a") as settings:
        settings.write("RALLY_WORKERS=7\n")
    output = subprocess.check_output(
        ["docker", "compose", "-p", "rally_release_test", "config", "--format", "json"],
        cwd=install,
        env=env,
        text=True,
    )
    services = json.loads(output)["services"]
    server = services["server"]
    assert server["image"] == f"{image}:{release}"
    assert "build" not in server
    assert "/opt/rally/runtime" not in json.dumps(server["volumes"])
    assert not (install / "runtime").exists()
    assert server["environment"]["RALLY_HOST_DATA"] == str(install / ".rally-data")
    assert str(server["environment"]["RALLY_WORKERS"]) == "7"
    assert services["db"]["image"] == "docker.io/library/postgres:17-alpine"
    print("Passed version-pinned source-free installation with optional server settings")
