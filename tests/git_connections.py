"""Exercise delegated HTTPS Git access through the same paths used by clients and workers."""

import json
import pathlib
import shlex
import shutil
import subprocess


def contract(s, wait):
    api = s.req
    url = s.forge.origin + "/project.git"
    repo = {"name": "project", "remote": url, "branch": "main"}
    alice = s.git_connection
    bob = json.loads(
        s.cli(
            "--json",
            "connection",
            "create",
            "--name",
            "Bob's forge",
            "--host",
            s.forge.origin,
            "--username",
            "bob",
            "--token-stdin",
            input="bob-fixture\n",
            env=dict(s.env, RALLY_TOKEN=s.bob),
        )
    )["id"]
    group = api("group.create", {"name": "Git contributors"})["id"]
    api("group.add", {"group": group, "person": s.bob_id})
    mid = api("mission.create", {"topic": "dev", "title": "Delegate a repository"})[
        "id"
    ]
    api("member.add", {"mission": mid, "principal": group})
    api("member.add", {"mission": mid, "principal": s.bob_id})
    binding = dict(repo, mission=mid, connection=bob)
    api("repo.add", binding, s.bob, denied=True)
    api("repo.add", dict(binding, confirm=True), denied=True)
    api("repo.add", dict(binding, confirm=True), s.bob)
    assert api("connection.list", token=s.bob)[0]["id"] == bob
    assert "bob-fixture" not in json.dumps(api("mission.show", {"mission": mid}))
    assert api("connection.usage", {"connection": bob}, s.bob)[0]["target"] == mid
    assert ("bob", "/project.git/git-upload-pack", "") in s.forge.calls
    assert (
        api("member.remove", {"mission": mid, "principal": s.bob_id, "preview": True})[
            "affected"
        ]
        == []
    )
    api("member.remove", {"mission": mid, "principal": s.bob_id})
    checkout = s.root / "data/missions" / mid / "project"
    (checkout / "keep").write_text("local work")
    impact = {"group": group, "person": s.bob_id}
    assert api("group.remove", dict(impact, preview=True))["affected"] == [
        mid + " / project"
    ]
    api("group.remove", impact, denied=True)
    api("group.remove", dict(impact, confirm=True))
    api("group.add", impact)
    assert api("repo.list", {"mission": mid})[0]["needs_transfer"]
    api("repo.push", {"mission": mid, "name": "project"}, denied=True)
    assert (checkout / "keep").read_text() == "local work"
    take = {"mission": mid, "name": "project", "connection": alice, "confirm": True}
    api("repo.takeover", take)
    assert (checkout / "keep").read_text() == "local work"
    fork = api("mission.fork", {"mission": mid, "title": "Independent branch"})["id"]
    assert api("repo.list", {"mission": fork})[0]["needs_transfer"]
    # A second contributor can push using the connection bound by the first
    s.git(checkout, "add", "keep")
    s.git(checkout, "commit", "-m", "Keep local work")
    api("repo.push", {"mission": mid, "name": "project"}, s.bob)
    assert ("alice", "/project.git/git-receive-pack", "") in s.forge.calls

    # The routine owns its workspace, while the dispatched agent uses the mission checkout
    title = "Created by the Git routine"
    repos = shlex.quote(json.dumps([repo]))
    owner_id = api("whoami")["id"]
    api("thread.subscribe", {"thread": mid, "mode": "all"})
    cmd = "\n".join(
        [
            "set -eu",
            "git -C project fetch origin",
            f"rally mission create --topic dev --title '{title}' --repos {repos} --confirm",
            f"rally repo add --mission {mid} --name routine-copy --remote {url} --branch main --confirm",
            f"rally post --silent --thread {mid} --mention {s.agent} --mention {owner_id} --confirm --body GIT_MISSION_PROBE",
        ]
    )
    routine = api(
        "routine.create",
        {
            "name": "Bob's Git routine",
            "command": cmd,
            "repos": [dict(repo, connection=bob)],
        },
        s.bob,
    )["id"]
    s.forge.calls.clear()
    api("routine.trigger", {"routine": routine}, s.bob)

    def job_done():
        jobs = api("routine.show", {"routine": routine}, s.bob)["jobs"]
        if jobs and jobs[0]["status"] in ("failed", "aborted"):
            raise AssertionError(jobs[0]["output"])
        return jobs and jobs[0]["status"] == "done"

    wait(job_done, "Git routine did not finish")
    created = next(m for m in api("mission.list", token=s.bob) if m["title"] == title)[
        "id"
    ]
    row = api("repo.list", {"mission": created}, s.bob)[0]
    assert row["contributor"] == s.bob_id and row["connection"] == bob
    assert (
        next(
            r for r in api("repo.list", {"mission": mid}) if r["name"] == "routine-copy"
        )["connection"]
        == bob
    )

    def read():
        return api("read", {"thread": mid})

    wait(
        lambda: any(
            m["body"] == "Mission Git access verified." for m in read()["messages"]
        ),
        "Silent prompt did not reach the hosted agent",
    )
    assert all("GIT_MISSION_PROBE" not in m["body"] for m in read()["messages"])
    assert routine not in [p["id"] for p in read()["participants"]]
    trigger = read()["runs"][0]["trigger"]
    assert not any(
        n.get("message") == trigger
        for n in api("inbox.list")
        if n["reason"] in ("mention", "following")
    )
    assert "GIT_MISSION_PROBE" not in s.cli("pull", mid)
    assert all(
        "GIT_MISSION_PROBE" not in hit["body"]
        for hit in api("search", {"query": "GIT_MISSION_PROBE"})
    )
    assert ("alice", "/project.git/git-receive-pack", "") in s.forge.calls
    assert ("bob", "/project.git/git-upload-pack", "") in s.forge.calls
    api("routine.transfer", {"routine": routine, "person": api("whoami")["id"]})
    transferred = api("routine.show", {"routine": routine})
    assert not transferred["enabled"] and transferred["repos"][0]["needs_connection"]
    api("routine.enable", {"routine": routine}, denied=True)
    api("routine.update", {"routine": routine, "repos": [dict(repo, connection=alice)]})
    api("routine.enable", {"routine": routine})

    # Each local checkout uses the developer's Git configuration independently
    shutil.copytree(s.root / "git/project.git", s.root / "git/public.git")
    api(
        "repo.add",
        dict(repo, mission=mid, name="public", remote=s.forge.origin + "/public.git"),
    )
    env = dict(
        s.env,
        GIT_CONFIG_COUNT="2",
        GIT_CONFIG_KEY_1=f"url.{s.root}/missing.git.insteadOf",
        GIT_CONFIG_VALUE_1=url,
    )
    opened = subprocess.run(
        ["rally", "open", mid, "--print-path"],
        env=env,
        cwd=s.root,
        capture_output=True,
        text=True,
        check=True,
    )
    assert "project" in opened.stderr and "Morgan Lee" in opened.stderr, opened.stderr
    assert (pathlib.Path(opened.stdout.strip()) / "public/.git").exists()

    rev = s.git(s.root / "developer", "rev-parse", "HEAD")
    api(
        "skill.import",
        {"name": "pinned-check", "remote": url, "ref": "absent", "connection": bob},
        s.bob,
        denied=True,
    )
    skill = api(
        "skill.import",
        {"name": "pinned-check", "remote": url, "ref": rev, "connection": bob},
        s.bob,
    )["id"]
    previous_fetch = api("skill.show", {"skill": skill})["last_fetch"]
    api(
        "connection.update",
        {"connection": bob, "token": "invalid-fixture-token"},
        s.bob,
    )
    wait(
        lambda: api("skill.show", {"skill": skill})["git_paused"],
        "Forge rejection did not pause sync",
        80,
    )
    assert api("skill.show", {"skill": skill})["revision"] == rev
    api("connection.update", {"connection": bob, "token": "bob-fixture"}, s.bob)
    wait(
        lambda: (
            api("skill.show", {"skill": skill})["last_fetch"] != previous_fetch
            and not api("skill.show", {"skill": skill})["sync_error"]
        ),
        "Token replacement did not resume sync",
        80,
    )
    api("connection.remove", {"connection": bob, "confirm": True}, s.bob)
    cached = api("skill.show", {"skill": skill}, s.bob)
    assert cached["git_paused"] and cached["revision"] == rev and cached["body"]
    api("skill.reconnect", {"skill": skill}, denied=True)
    assert api("skill.show", {"skill": skill})["git_paused"]
    api("skill.reconnect", {"skill": skill, "connection": alice})
    assert not api("skill.show", {"skill": skill})["git_paused"]
    assert api("repo.list", {"mission": created}, s.bob)[0]["needs_transfer"]
    print(
        "Passed personal Git connections, effective removal, takeover, forks, partial local open, "
        "routine ownership, silent dispatch and pinned skill reconnection",
        flush=True,
    )
