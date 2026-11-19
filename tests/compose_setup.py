"""Verify host config selection with the same podman-compose resolver used at startup."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import yaml


root = Path(__file__).resolve().parents[1]
env = {k: v for k, v in os.environ.items() if not k.startswith(("RALLY_", "COMPOSE_"))}
with tempfile.TemporaryDirectory(prefix="rally setup ") as tmp:
    checkout = Path(tmp)
    shutil.copy(root / "compose.yml", checkout)
    shutil.copytree(root / "examples", checkout / "examples")

    def setup():
        subprocess.run(
            ["sh", root / "scripts/setup-compose.sh"], cwd=checkout, env=env, check=True
        )

    def compose():
        return subprocess.run(
            ["podman-compose", "-p", "rally_setup_test", "config"],
            cwd=checkout,
            env=env,
            capture_output=True,
            text=True,
        )

    setup()
    dotenv = checkout / ".env"
    config = checkout / ".rally-data/config.toml"
    assert config.read_bytes() == (checkout / "examples/config.toml").read_bytes()
    result = compose()
    result.check_returncode()
    server = yaml.safe_load(result.stdout)["services"]["server"]
    mount = next(
        v
        for v in server["volumes"]
        if isinstance(v, dict) and v["source"] == str(config)
    )
    assert mount["read_only"] and mount["bind"]["create_host_path"] is False
    assert server["entrypoint"] == ["rally-server", "--codex-config", mount["target"]]

    legacy = "".join(
        line
        for line in dotenv.read_text().splitlines(True)
        if not line.startswith("RALLY_CODEX_CONFIG=")
    )
    for setting in ("", "RALLY_CODEX_CONFIG=\n"):
        dotenv.write_text(legacy + setting)
        result = compose()
        assert (
            result.returncode != 0
            and "Run sh scripts/setup-compose.sh" in result.stderr
        )

    dotenv.write_text(legacy)
    custom = 'model = "custom"\n'
    config.write_text(custom)
    setup()
    repaired = dotenv.read_bytes()
    assert repaired.decode().splitlines() == [
        *legacy.splitlines(),
        "",
        f"RALLY_CODEX_CONFIG={config}",
    ]
    setup()
    assert dotenv.read_bytes() == repaired and config.read_text() == custom
    print(
        "Passed fresh setup, missing and empty settings, existing config preservation, and repeat setup"
    )
