"""Exercise the HTTP to script to CLI round trip on a running Rally Compose deployment."""

import json
import os
import time
import urllib.request
from urllib.parse import urlsplit

base = os.environ["RALLY_URL"].rstrip("/")
token = os.environ.get("RALLY_TOKEN")


def request(path, data, bearer=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {bearer or token}"
    req = urllib.request.Request(base + path, json.dumps(data).encode(), headers)
    with urllib.request.urlopen(req, timeout=60) as res:
        return json.load(res)


if not token:
    token = request(
        "/auth/login",
        {
            "email": os.environ["RALLY_TEST_EMAIL"],
            "password": os.environ["RALLY_TEST_PASSWORD"],
            "cli": True,
        },
    )["token"]

slug = f"ops{int(time.time())}"
request("/api/topic.create", {"slug": slug, "name": "Runtime verification"})
mission = request(
    "/api/mission.create",
    {"topic": slug, "title": "Verify the isolated routine workspace"},
)["id"]
command = (
    "set -eu\n"
    "test ! -S /run/rally/podman.sock\n"
    "test -f event.json\n"
    "rally whoami\n"
    f"rally post --thread {mission} --body 'The routine reported from its isolated container.'\n"
)
hook=request("/api/hook.create",{"name":"Compose events"})
credential=request("/api/hook.token",{"hook":hook["id"],"name":"Integration sender"})
routine = request("/api/routine.create", {"name": "Runtime check", "command": command, "triggers":[{"webhook":hook["id"]}]})
job = request(urlsplit(hook["url"]).path, {"event": "compose.integration"},credential["token"])["jobs"][0]
for _ in range(90):
    jobs = request("/api/routine.show", {"routine": routine["id"]})["jobs"]
    record = next((row for row in jobs if row["id"] == job), None)
    if record and record["status"] in {"done", "failed", "aborted"}:
        assert record["status"] == "done", record["output"]
        break
    time.sleep(1)
else:
    raise AssertionError("Routine did not finish")
messages = request("/api/read", {"thread": mission})["messages"]
assert messages[-1]["source"] == "routine" and messages[-1]["author"] == "Runtime check"
print("Passed: webhook → Podman runtime → authenticated CLI → mission conversation")
