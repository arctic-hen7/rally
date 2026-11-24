"""Check routed transport with real Git clients and independently delegated HTTPS credentials."""

import base64
import json
import os
import pathlib
import shutil
import subprocess
import urllib.error
import urllib.request


def worker_probe(data):
    fixture = data.get("fixture", "dependency.git")
    remote = data["origin"] + "/" + fixture
    aliases = [
        remote,
        "git@git.route.test:team/" + fixture,
        "ssh://git@git.route.test/team/" + fixture,
    ]

    def fetch(url):
        return subprocess.run(
            ["git", "ls-remote", "--exit-code", "--", url, "HEAD"], capture_output=True
        ).returncode

    if data["mode"] == "missing":
        assert all(fetch(url) != 0 for url in aliases)
        return "Read requests checked"

    if fixture == "dependency.git":
        aliases.append("git@git.route.test:team/renamed.git")
    for url in aliases:
        assert fetch(url) == 0, url

    def route(url):
        return "@routes/" + base64.urlsafe_b64encode(url.encode()).decode().rstrip("=")

    probes = [data["name"], route(remote), route(data["origin"] + "/ungranted.git")]
    operations = [("GET", "info/refs?service=git-receive-pack"), ("POST", "git-receive-pack")]
    auth = os.environ["GIT_CONFIG_VALUE_0"].removeprefix("Authorization: ")
    for path in probes:
        for method, rpc in operations:
            request = urllib.request.Request(
                os.environ["RALLY_GIT_PROXY"] + "/" + path + "/" + rpc,
                method=method,
                headers={"Authorization": auth},
            )
            try:
                urllib.request.urlopen(request)
            except urllib.error.HTTPError as err:
                assert err.code == 403
            else:
                raise AssertionError("Git broker authorised a write")
    return "Read-only transport checked"


def contract(s, wait):
    api = s.req
    origin = s.forge.origin
    remote = origin + "/dependency.git"
    shutil.copytree(s.root / "git/project.git", s.root / "git/dependency.git")
    scope = {"source": "git@git.route.test:team/", "scope": "prefix", "upstream": origin}
    api("routing.create", scope, s.bob, denied=True)
    rule = api("routing.create", scope)["id"]
    narrower = api("routing.create", {
        "source": "git@git.route.test:team/renamed.git",
        "upstream": remote,
        "scope": "repository",
    })["id"]
    api("routing.create", dict(scope, source="https://other.test", upstream="http://other.test"), denied=True)
    mid = api("mission.create", {"title": "Routed dependency checks", "topic": "dev"})["id"]
    api("member.add", {"mission": mid, "principal": s.bob_id})
    data = {"mission": mid, "remote": remote}

    def dispatch(mode, **kwargs):
        payload = json.dumps(dict(mode=mode, origin=origin, **kwargs))
        run = api("post", {
            "thread": mid,
            "body": "ROUTING_FIXTURE " + payload,
            "mentions": [s.agent],
            "confirm": True,
        })["runs"][0]

        def finished():
            state = api("run.show", {"run": run})
            assert state["status"] != "failed", state.get("error")
            return state["status"] == "done"

        wait(finished, "Git routing fixture did not finish")
        return api("read", {"thread": mid})["messages"][-1]["body"]

    assert dispatch("missing") == "Read requests checked"
    assert json.loads(s.cli("--json", "repo", "requests", "--mission", mid)) == [remote]
    assert rule in [r["id"] for r in json.loads(s.cli("--json", "routing", "list"))]
    for token in [s.token, s.bob]:
        alerts = [n for n in api("inbox.list", token=token) if n["mission"] == mid and n["reason"] == "git-access"]
        assert len(alerts) == 1
    api("repo.dismiss", data)
    assert dispatch("missing") == "Read requests checked"
    assert not api("repo.requests", data)
    grant = dict(data, connection=s.git_connection)
    api("repo.attach", grant, denied=True)
    api("repo.attach", dict(grant, confirm=True), s.bob, denied=True)
    name = api("repo.attach", dict(grant, confirm=True))["name"]
    s.forge.calls.clear()
    assert dispatch("attached", name=name) == "Read-only transport checked"
    assert any(u == "alice" and p == "/dependency.git/git-upload-pack" for u, p, q in s.forge.calls)
    assert all("receive-pack" not in (p + q) for u, p, q in s.forge.calls)
    show = api("mission.show", data)
    assert not show["repos"] and show["read_only_repos"][0]["name"] == name
    assert not (s.root / "data/missions" / mid / name).exists()
    local = pathlib.Path(s.cli("open", mid, "--print-path"))
    assert not (local / name).exists() and not list(local.glob("*/.git"))
    for op in ["repo.wip", "repo.push", "repo.return"]:
        api(op, dict(data, name=name), denied=True)
    api("repo.takeover", dict(grant, name=name, confirm=True, read_only=False, branch="main"))
    assert api("repo.list", data)[0]["read_only"]
    fork = api("mission.fork", {"mission": mid, "title": "Fork with a dependency"})["id"]
    dep = api("repo.list", {"mission": fork})[0]
    assert dep["read_only"] and dep["needs_transfer"] and dep["branch"] is None
    api("repo.withdraw", dict(data, name=name))
    assert dispatch("missing") == "Read requests checked"
    assert not api("repo.requests", data)
    api("repo.add", dict(grant, name="dependency", branch="main", confirm=True))
    assert len(api("repo.list", data)) == 1 and not api("repo.list", data)[0]["read_only"]
    assert (s.root / "data/missions" / mid / "dependency/.git").exists()

    public = dict(data, remote=origin + "/public.git")
    assert dispatch("missing", fixture="public.git") == "Read requests checked"
    assert api("repo.requests", data) == [public["remote"]]
    anonymous = api("repo.attach", public)["name"]
    assert dispatch("attached", fixture="public.git", name=anonymous) == "Read-only transport checked"
    assert not api("repo.requests", data)
    api("repo.remove", dict(data, name=anonymous))
    job = api("routine.create", {
        "name": "Routed Git job",
        "repos": [dict(name="dep", remote=remote, branch="main", connection=s.git_connection)],
        "command": "set -eu\ngit ls-remote git@git.route.test:team/dependency.git HEAD\n"
                   "! git ls-remote git@git.route.test:team/unlisted.git HEAD",
    })["id"]
    api("routine.trigger", {"routine": job})

    def job_done():
        jobs = api("routine.show", {"routine": job})["jobs"]
        assert not jobs or jobs[0]["status"] != "failed", jobs
        return jobs and jobs[0]["status"] == "done"

    wait(job_done, "Routed Git job did not finish")
    api("routing.update", dict(scope, id=rule, source="ssh://git@git.route.test:2222/team/"))
    api("routing.remove", {"id": narrower})
    api("routing.remove", {"id": rule})
    print("Passed hosted SSH, SCP and HTTPS routes, isolated routine access, read-only grants and local open", flush=True)
