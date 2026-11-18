"""Check a real hosted harness using the offline HTTP provider in tests/provider.py."""

import json
import os
from pathlib import Path
import time
import urllib.request

base = os.environ["RALLY_URL"]
token = ""


def api(path, data):
    request = urllib.request.Request(
        base + path,
        json.dumps(data).encode(),
        {"Content-Type": "application/json", "Authorization": "Bearer " + token},
    )
    return json.load(urllib.request.urlopen(request, timeout=30))


token = api(
    "/auth/login",
    {
        "email": os.environ["RALLY_TEST_EMAIL"],
        "password": os.environ["RALLY_TEST_PASSWORD"],
        "cli": True,
    },
)["token"]
suffix = str(int(time.time()))
topic = api(
    "/api/topic.create", {"slug": "live" + suffix, "name": "Hosted runtime " + suffix}
)["slug"]
mission = api(
    "/api/mission.create", {"topic": topic, "title": "Verify native runtime continuity"}
)["id"]
agent = api(
    "/api/agent.create",
    {
        "name": "runtime-" + suffix,
        "model": "gpt-6-luna",
        "instructions": "Reply concisely.",
    },
)["id"]
session = None
for message in [
    "RALLY_WORKSPACE_PROBE: verify a workspace edit.",
    "Acknowledge this follow-up in the same session.",
]:
    rid = api(
        "/api/post",
        {"thread": mission, "body": message, "mentions": [agent], "confirm": True},
    )["runs"][0]
    for _ in range(100):
        run = api("/api/run.show", {"run": rid})
        if run["status"] in {"failed", "stopped", "done"}:
            assert run["status"] == "done", run["error"]
            assert "container runtime verified" in run["partial"], run["partial"]
            if session:
                assert session == run["session"]
            if "RALLY_TEST_WORKSPACES" in os.environ:
                path = (
                    Path(os.environ["RALLY_TEST_WORKSPACES"])
                    / mission
                    / "rally-probe.txt"
                )
                assert path.read_text() == "verified"
            session = run["session"]
            break
        time.sleep(0.7)
    else:
        raise AssertionError("Hosted turn timed out")
print(
    "Passed: real container harness performs two turns through the offline provider in one session"
)
