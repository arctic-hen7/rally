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

    if data["mode"] == "http":
        aliases = [remote, "git@code.example.test:team/" + fixture, "https://code.example.test/team/" + fixture]
    elif fixture == "dependency.git":
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
    api("routing.create", dict(scope, source="https://other.test", upstream="ftp://other.test"), denied=True)
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


def http_rewrites(s, wait):
    from git_forge import Forge
    forge = Forge(s.root, tls=False)
    try:
        check_http_rewrites(s, forge, wait)
    finally:
        forge.close()


def check_http_rewrites(s, forge, wait):
    api = s.req
    prefix = "https://code.example.test/team/"
    source = prefix + "project.git"
    upstream = forge.origin + "/project.git"
    cid = api("connection.create", {
        "name": "Internal Git", "host": forge.origin, "username": "alice", "token": "alice-fixture",
    })["id"]
    mid = api("mission.create", {"topic": "dev", "title": "Hosted HTTP transport"})["id"]
    binding = dict(mission=mid, remote=source, name="project", branch="main", connection=cid, confirm=True)
    api("repo.attach", dict(binding, remote=upstream), denied=True)
    rule = api("routing.create", {"source": prefix, "upstream": forge.origin, "scope": "prefix"})["id"]
    assert api("routing.resolve", {"remote": "git@code.example.test:team/project.git"})["upstream"] == upstream
    tls_only = api("connection.create", {
        "name": "Same port over TLS", "host": forge.origin.replace("http:", "https:"),
        "username": "alice", "token": "alice-fixture",
    })["id"]
    api("repo.add", dict(binding, connection=tls_only), denied=True)
    api("repo.add", binding)
    checkout = s.root / "data/missions" / mid / "project"
    skill_dir = checkout / "http-fixture"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text("---\nname: http-fixture\ndescription: Transport check\n---\nRead the fixture.\n")
    s.git(checkout, "add", "http-fixture/SKILL.md")
    s.git(checkout, "commit", "-m", "Exercise HTTP push")
    api("repo.push", binding)
    assert ("alice", "/project.git/git-receive-pack", "") in forge.calls
    row = api("mission.show", binding)["repos"][0]
    assert row["remote"] == source and row["upstream"] == upstream
    local_env = dict(s.env, GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0=f"url.{s.root}/git/.insteadOf",
                     GIT_CONFIG_VALUE_0=prefix)
    local = pathlib.Path(s.cli("open", mid, "--print-path", env=local_env))
    assert s.git(local / "project", "config", "--get", "remote.origin.url") == source

    ssh = "git@code.example.test:team/project.git"
    skill = api("skill.import", dict(name="http-fixture", remote=ssh, path="http-fixture", connection=cid))["id"]
    assert "Read the fixture" in api("skill.show", {"skill": skill})["body"]
    job = api("routine.create", {
        "name": "Internal HTTP workspace",
        "repos": [dict(name="project", remote=ssh, branch="main", connection=cid)],
        "command": "git ls-remote --exit-code https://code.example.test/team/project.git HEAD",
    })["id"]
    api("routine.trigger", {"routine": job})

    def job_done():
        jobs = api("routine.show", {"routine": job})["jobs"]
        assert not jobs or jobs[0]["status"] != "failed", jobs
        return jobs and jobs[0]["status"] == "done"

    wait(job_done, "HTTP routine fetch did not finish")
    dep = forge.origin + "/dependency.git"
    readonly = api("repo.attach", dict(binding, remote=dep))["name"]
    payload = json.dumps(dict(mode="http", fixture="dependency.git", origin=forge.origin, name=readonly))
    rid = api("post", {"thread": mid, "body": "ROUTING_FIXTURE " + payload,
                       "mentions": [s.agent], "confirm": True})["runs"][0]
    def done():
        state = api("run.show", {"run": rid})
        assert state["status"] != "failed", state.get("error")
        return state["status"] == "done"
    wait(done, "Hosted HTTP dependency probe did not finish")
    assert all("receive-pack" not in (path + query) for _, path, query in forge.calls if "dependency.git" in path)

    api("routing.remove", {"id": rule})
    before = len(forge.requests)
    api("repo.push", binding, denied=True)
    s.forge.redirects["/redirect.git/info/refs"] = upstream + "/info/refs?service=git-upload-pack"
    api("repo.add", dict(binding, name="redirect", remote=s.forge.origin + "/redirect.git",
                         connection=s.git_connection), denied=True)
    assert "/redirect.git/info/refs" in s.forge.requests
    api("repo.add", dict(binding, name="tls", remote=upstream.replace("http:", "https:"), connection=None), denied=True)
    assert len(forge.requests) == before
    print("Passed explicit HTTP rewrites, local clone URLs, read-only delegation and redirect/TLS rejection", flush=True)
