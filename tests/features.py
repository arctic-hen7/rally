"""Integration coverage for resource sharing, scoped environments and provider credentials."""

import concurrent.futures
import http.server
import json
import os
import pathlib
import pty
import subprocess
import threading
import time
import urllib.error
import urllib.request


def secrets_and_access(s):
    def apply(target, key, value, token=None, old=None):
        old = old or {}
        return s.req(
            "secrets.apply",
            {
                "target": target,
                "changes": {
                    key: {
                        "base": old.get("checksum"),
                        "source": old.get("source"),
                        "value": value,
                    }
                },
            },
            token,
        )

    private = s.req(
        "agent.create", {"name": "scheduler", "model": "test", "private": True}
    )["id"]
    skill = s.req(
        "skill.create",
        {
            "name": "calendar",
            "description": "Calendar access",
            "body": "Use the calendar.",
            "private": True,
        },
    )["id"]
    other = s.req(
        "mission.create",
        {"topic": s.mid.rsplit("-", 1)[0], "title": "Independent work"},
    )["id"]
    s.req("member.add", {"mission": other, "principal": s.bob_id})
    assert private not in [a["id"] for a in s.req("agent.list", token=s.bob)]
    s.req("agent.show", {"agent": private}, s.bob, denied=True)
    s.req("skill.show", {"skill": skill}, s.bob, denied=True)
    s.req(
        "member.add",
        {"mission": s.mid, "principal": private, "confirm": True},
        s.bob,
        denied=True,
    )
    s.req(
        "post",
        {"thread": s.mid, "mentions": [private], "confirm": True, "body": "Invite"},
        s.bob,
        denied=True,
    )
    s.req("skill.attach", {"target": private, "skill": skill}, denied=True)
    s.req("skill.attach", {"target": private, "skill": skill, "confirm": True})
    s.req("resource.privacy", {"target": private, "private": False}, denied=True)
    s.req("resource.privacy", {"target": private, "private": False, "confirm": True})
    s.req("resource.privacy", {"target": private, "private": True})
    s.req("member.add", {"mission": s.mid, "principal": private, "confirm": True})
    assert private in [a["id"] for a in s.req("agent.list", token=s.bob)]
    s.req(
        "member.add",
        {"mission": other, "principal": private, "confirm": True},
        s.bob,
        denied=True,
    )
    group = s.req("group.list")[0]["id"]
    s.req("resource.privacy", {"target": private, "invites": [group]})
    s.req(
        "member.add", {"mission": other, "principal": private, "confirm": True}, s.bob
    )
    s.req("resource.privacy", {"target": private, "invites": []})
    assert {
        a["id"] for a in s.req("resource.privacy", {"target": private})["targets"]
    } == {s.mid, other}
    s.req("resource.privacy", {"target": skill, "invites": [s.bob_id]})
    s.req("skill.attach", {"target": s.mid, "skill": skill, "confirm": True}, s.bob)
    assert {
        a["id"] for a in s.req("resource.privacy", {"target": skill})["targets"]
    } == {s.mid, private}

    apply(skill, "CALENDAR_TOKEN", "inherited")
    apply(private, "ONLY_ROLE", "role")
    s.req("secrets.list", {"target": private}, s.bob, denied=True)
    apply(s.mid, "MISSION_SAMPLE", "first", s.bob)
    visible = s.req("secrets.list", {"mission": s.mid}, s.bob)
    assert "ONLY_ROLE" not in visible
    assert visible["CALENDAR_TOKEN"]["source"] == skill

    workspace = pathlib.Path(s.cli("open", s.mid, "--print-path"))
    dotenv = workspace / ".rally.env"
    assert dotenv.stat().st_mode & 0o777 == 0o600
    s.cli("secrets", "pull", "--mission", s.mid)
    assert "# Skill: calendar" in dotenv.read_text()
    dotenv.write_text(
        dotenv.read_text().replace('"inherited"', '"override"')
        + 'NEW_KEY="two words"\n'
    )
    s.cli("secrets", "push", "--mission", s.mid)
    entries = s.req("secrets.list", {"mission": s.mid})
    assert entries["NEW_KEY"]["source"] == entries["CALENDAR_TOKEN"]["source"] == s.mid
    assert (
        s.req("secrets.list", {"target": skill})["CALENDAR_TOKEN"]["value"]
        == "inherited"
    )
    old = entries["MISSION_SAMPLE"]
    apply(s.mid, "MISSION_SAMPLE", "second", old=old)
    dotenv.write_text(
        dotenv.read_text().replace('"first"', '"my edit"') + "OTHER=one\n"
    )
    try:
        s.cli("secrets", "push", "--mission", s.mid)
        raise AssertionError("Concurrent change was overwritten")
    except subprocess.CalledProcessError:
        pass
    assert "OTHER" not in s.req("secrets.list", {"mission": s.mid})
    s.cli("secrets", "push", "--mission", s.mid, "--overwrite", "MISSION_SAMPLE")
    assert (
        s.req("secrets.list", {"mission": s.mid})["MISSION_SAMPLE"]["value"]
        == "my edit"
    )
    apply(
        s.mid,
        "MISSION_SAMPLE",
        "first",
        old=s.req("secrets.list", {"mission": s.mid})["MISSION_SAMPLE"],
    )

    thread = s.req("thread.create", {"mission": s.mid, "title": "Secret scope probe"})[
        "id"
    ]
    run = s.req("post", {"thread": thread, "mentions": [private], "body": "HOLD"})[
        "runs"
    ][0]
    until(lambda: "Working." in s.req("run.show", {"run": run})["partial"])
    apply(
        s.mid,
        "MISSION_SAMPLE",
        "second",
        old=s.req("secrets.list", {"mission": s.mid})["MISSION_SAMPLE"],
    )
    s.req("run.steer", {"run": run, "body": "CHECK_ENV"})
    until(lambda: s.req("run.show", {"run": run})["status"] == "done")
    assert "scope=first,role,inherited" in s.req("run.show", {"run": run})["partial"]
    run = s.req("post", {"thread": thread, "mentions": [private], "body": "CHECK_ENV"})[
        "runs"
    ][0]
    until(lambda: s.req("run.show", {"run": run})["status"] == "done")
    assert "scope=second,role,inherited" in s.req("run.show", {"run": run})["partial"]
    s.req("member.add", {"mission": s.mid, "principal": s.bob_id})
    assert (
        "Developers"
        in s.req("member.remove", {"mission": s.mid, "principal": s.bob_id})[
            "remaining"
        ]
    )
    s.req("member.remove", {"mission": other, "principal": s.bob_id})
    s.req("secrets.list", {"mission": other}, s.bob, denied=True)
    print(
        "Passed: private grants and retained attachments, effective membership, scoped launch environments, local overrides and atomic checksum conflicts",
        flush=True,
    )


def until(fn, timeout=25):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if fn():
            return
        time.sleep(0.12)
    raise AssertionError("Expected state was not reached")


def trigger_contract(s):
    hook = s.req("hook.create", {"name": "Multi-sender"})
    token = lambda name: s.req("hook.token", {"hook": hook["id"], "name": name})
    first, second = token("CI"), token("Calendar")

    def routine(name):
        return s.req(
            "routine.create",
            {"name": name, "command": "true", "triggers": [{"webhook": hook["id"]}]},
        )["id"]

    r1, r2 = routine("Review"), routine("Publish")
    s.req("hook.show", {"hook": hook["id"]}, s.bob, denied=True)
    s.req(
        "routine.create",
        {"name": "Forbidden", "command": "true", "triggers": [{"webhook": hook["id"]}]},
        s.bob,
        denied=True,
    )

    def deliver(credential):
        request = urllib.request.Request(
            hook["url"],
            data=b"{}",
            headers={
                "Content-Type": "application/json",
                "Authorization": "Bearer " + credential,
                "Idempotency-Key": "build-123",
            },
        )
        with urllib.request.urlopen(request) as resp:
            assert resp.status == 202
            return json.load(resp)

    assert len(deliver(first["token"])["jobs"]) == 2
    assert deliver(first["token"])["duplicate"]
    s.req("hook.revoke", {"hook": hook["id"], "token": first["id"]})
    try:
        deliver(first["token"])
        raise AssertionError("Revoked token was accepted")
    except urllib.error.HTTPError as e:
        assert e.code == 401
    assert len(deliver(second["token"])["jobs"]) == 2
    assert not s.req("routine.transfer-preview", {"routine": r1})["hooks"][0][
        "exclusive"
    ]
    s.req("routine.transfer", {"routine": r1, "person": s.bob_id}, denied=True)
    s.req(
        "routine.transfer",
        {"routine": r1, "person": s.bob_id, "confirm": True, "handover": True},
    )
    assert s.req("routine.show", {"routine": r1}, s.bob)["triggers"] == []
    s.req(
        "routine.transfer",
        {"routine": r2, "person": s.bob_id, "confirm": True, "handover": True},
    )
    s.req("hook.show", {"hook": hook["id"]}, denied=True)
    assert s.req("hook.show", {"hook": hook["id"]}, s.bob)["tokens"][0]["last_used"]
    assert len(s.req("/hooks/" + hook["id"], {}, second["token"])["jobs"]) == 1
    assert (
        len(
            s.req(
                "routine.preview",
                {"cron": "0 9 * * MON-FRI", "timezone": "Australia/Sydney"},
            )
        )
        == 3
    )
    print(
        "Passed: webhook isolation, fan-out, idempotency, revocation, timezone previews, shared detachment and exclusive handover",
        flush=True,
    )


def provider_contract(s, binary, root):
    s.stop()
    opts = ["--codex-config", str(root / "tests/provider.toml")]
    default = pathlib.Path(s.env["RALLY_DATA"]) / "codex/auth.json"

    def command(args, env=None):
        return subprocess.run(
            [binary, *args],
            env=env or s.env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=10,
        )

    assert command([]).returncode != 0
    assert "Missing Codex login" in command(opts).stderr
    assert (
        "Read Codex credentials"
        in command([*opts, "--auth", str(s.root / "absent")]).stderr
    )
    fake = s.root / "codex-login"
    fake.write_text(
        '#!/bin/sh\nset -eu\nprintf \'%s\' \'{"OPENAI_API_KEY":"fixture-key"}\' > "$CODEX_HOME/auth.json"\n'
    )
    fake.chmod(0o700)
    loginenv = dict(s.env, RALLY_CODEX=str(fake))
    primary, terminal = pty.openpty()
    try:
        proc = subprocess.Popen(
            [binary, *opts], env=loginenv, stdin=terminal, stdout=s.log, stderr=s.log
        )
        try:

            def ready():
                if proc.poll() is not None:
                    raise AssertionError("Implicit interactive login failed")
                try:
                    with urllib.request.urlopen(
                        s.url + "/health", timeout=1
                    ) as response:
                        return response.status == 200
                except OSError:
                    return False

            until(ready)
            assert s.req("models.settings")["provider"]["mode"] == "managed"
        finally:
            proc.terminate()
            proc.wait(timeout=25)
    finally:
        os.close(terminal)
        os.close(primary)
    assert command([*opts, "login"], loginenv).returncode == 0
    assert default.stat().st_mode & 0o777 == 0o600
    assert command([*opts, "login", "status"]).returncode == 0
    assert (
        command(
            [
                *opts,
                "--auth",
                str(s.root / "absent"),
                "--bypass-auth",
                "login",
                "status",
            ]
        ).returncode
        == 0
    )

    class Refresh(http.server.BaseHTTPRequestHandler):
        requests = 0

        def do_POST(self):
            d = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            time.sleep(0.4)
            if d["refresh_token"] == "new-fixture-refresh":
                self.send_response(401)
                self.end_headers()
                return
            assert d["refresh_token"] == "initial-fixture-refresh"
            type(self).requests += 1
            self.send_response(200)
            self.end_headers()
            self.wfile.write(
                b'{"access_token":"refreshed-test-access","refresh_token":"new-fixture-refresh"}'
            )

        def log_message(self, *args):
            pass

    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Refresh) as issuer:
        threading.Thread(target=issuer.serve_forever, daemon=True).start()
        source = s.root / "selected.json"
        source.write_text(
            json.dumps(
                {
                    "tokens": {
                        "account_id": "test",
                        "access_token": "initial-fixture-access",
                        "refresh_token": "initial-fixture-refresh",
                        "oauth_provenance": {
                            "issuer": "http://127.0.0.1:" + str(issuer.server_port),
                            "client_id": "fixture",
                        },
                    }
                }
            )
        )
        s.server(["--auth", str(source)])
        assert s.req("models.settings")["provider"]["mode"] == "managed"

        def dispatch(i):
            thread = s.req(
                "thread.create",
                {"mission": s.mid, "title": "Credential refresh " + str(i)},
            )["id"]
            rid = s.req(
                "post",
                {
                    "thread": thread,
                    "body": "HOLD",
                    "mentions": [s.agent],
                    "confirm": True,
                },
            )["runs"][0]
            until(lambda: "Working." in s.req("run.show", {"run": rid})["partial"])
            return rid

        runs = [dispatch(i) for i in range(2)]
        with concurrent.futures.ThreadPoolExecutor() as pool:
            list(
                pool.map(
                    lambda rid: s.req(
                        "run.steer", {"run": rid, "body": "REFRESH_AUTH"}
                    ),
                    runs,
                )
            )
        until(
            lambda: all(
                s.req("run.show", {"run": rid})["status"] == "done" for rid in runs
            )
        )
        assert Refresh.requests == 1
        assert (
            json.loads(source.read_text())["tokens"]["refresh_token"]
            == "new-fixture-refresh"
        )
        assert json.loads(default.read_text())["OPENAI_API_KEY"] == "fixture-key"
        expired = dispatch("expired")
        s.req("run.steer", {"run": expired, "body": "REFRESH_AUTH"})
        until(lambda: s.req("run.show", {"run": expired})["status"] == "failed")
        assert "Reauthenticate" in s.req("models.settings")["provider"]["error"]
        assert (
            json.loads(source.read_text())["tokens"]["refresh_token"]
            == "new-fixture-refresh"
        )
        issuer.shutdown()
    s.stop()
    s.server()
    print(
        "Passed: mandatory configuration, missing-login errors, private login extraction, bypass precedence, native handoff and concurrent refresh against the selected authority",
        flush=True,
    )
