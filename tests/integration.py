"""Run against a temporary database created on the supplied PostgreSQL instance."""

import base64
import json
import http.cookiejar
from oidc_fixture import Issuer
import os
import pathlib
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from contextlib import closing

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = pathlib.Path(os.environ.get("RALLY_BIN_DIR", ROOT / "target/debug")).resolve()
OWNER = "Morgan Lee"
PASSWORD = "synthetic-rally-password"
EMAIL = "morgan@rally.test"


def wait(check, message, timeout=35):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = check()
        if result:
            return result
        time.sleep(0.15)
    raise AssertionError(message)


def free_port():
    with closing(socket.socket()) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Suite:
    def __init__(self, root, database):
        self.root = root
        self.root.mkdir(exist_ok=True)
        self.url = "http://127.0.0.1:" + str(free_port())
        self.env = dict(
            os.environ,
            RALLY_URL=self.url,
            RALLY_BIND=self.url.split("//")[1],
            RALLY_ENGINE="local",
            RALLY_ADAPTER="codex",
            CODEX_HOME=str(root / "home/.codex"),
            CLAUDE_CONFIG_DIR=str(root / "home/.claude"),
            RALLY_DATA=str(root / "data"),
            RALLY_CODEX=str(ROOT / "tests/harness.py"),
            RALLY_CONFIG_DIR=str(root / "profile"),
            RALLY_BOOTSTRAP_SECRET=secrets.token_hex(16),
            DATABASE_URL=database,
            PATH=str(BIN) + ":" + os.environ["PATH"],
            RALLY_WORKER_URL=self.url,
            RALLY_WEB_DIR=str(ROOT / "web/dist"),
            HOME=str(root / "home"),
        )
        self.env.pop("RALLY_TOKEN", None)
        self.env.pop("RALLY_MODEL_PROVIDER", None)
        self.env["GIT_CONFIG_COUNT"] = "1"
        self.env["GIT_CONFIG_KEY_0"] = f"url.{root}/git/.insteadOf"
        self.env["GIT_CONFIG_VALUE_0"] = "https://rally.invalid/"
        self.env["GIT_ALLOW_PROTOCOL"] = "file:https:ssh"
        self.issuer = Issuer(root)
        self.env.update(RALLY_OIDC_ISSUER=self.issuer.url, RALLY_OIDC_CLIENT_ID="rally")
        self.process = None
        self.token = None
        self.log = open(root / "server.log", "ab")

    def server(self):
        self.process = subprocess.Popen(
            [BIN / "rally-server"], env=self.env, stdout=self.log, stderr=self.log
        )

        def ready():
            if self.process.poll() is not None:
                raise AssertionError((self.root / "server.log").read_text()[-3000:])
            try:
                return (
                    urllib.request.urlopen(self.url + "/health", timeout=1).status
                    == 200
                )
            except OSError:
                return False

        wait(ready, "Server never became healthy")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=25)

    def req(self, op, data=None, token=None, denied=False):
        headers = {"Content-Type": "application/json"}
        token = self.token if token is None else token
        if token:
            headers["Authorization"] = "Bearer " + token
        path = op if op.startswith("/") else "/api/" + op
        r = urllib.request.Request(
            self.url + path, json.dumps(data or {}).encode(), headers
        )
        try:
            with urllib.request.urlopen(r, timeout=90) as response:
                result = json.load(response)
        except urllib.error.HTTPError as e:
            detail = json.load(e)
            if denied:
                return detail
            raise AssertionError(f"{op}: {e.code} {detail}") from e
        assert not denied, f"{op} unexpectedly succeeded"
        return result

    def cli(self, *args, cwd=None, env=None, input=None):
        return subprocess.check_output(
            [BIN / "rally", *args],
            cwd=cwd or self.root,
            env=env or self.env,
            input=input,
            text=True,
        ).strip()

    def git(self, cwd, *args):
        return subprocess.check_output(
            [
                "git",
                "-c",
                "user.name=Rally Test",
                "-c",
                "user.email=test@rally.invalid",
                "-c",
                "commit.gpgsign=false",
                *args,
            ],
            cwd=cwd,
            env=self.env,
            text=True,
            stderr=subprocess.STDOUT,
        ).strip()

    def auth(self):
        self.token = self.req(
            "/auth/bootstrap",
            {
                "email": EMAIL,
                "password": PASSWORD,
                "name": OWNER,
                "code": self.env["RALLY_BOOTSTRAP_SECRET"],
                "cli": True,
            },
        )["token"]
        self.cli("login", "--token", self.token)
        self.req("topic.create", {"slug": "dev", "name": "Engineering"})
        self.mid = self.req(
            "mission.create", {"topic": "dev", "title": "Implement shared search"}
        )["id"]
        self.secret = self.req(
            "mission.create", {"topic": "dev", "title": "Investigate contractor budget"}
        )["id"]
        self.req(
            "post",
            {
                "thread": self.secret,
                "body": "privatebudgetmarker covers contractor hiring projections",
            },
        )
        invite = self.req("account.invite", {"email": "bob@rally.test"})
        self.bob = self.req(
            "/auth/register",
            {
                "email": "bob@rally.test",
                "name": "Bob",
                "password": PASSWORD,
                "code": invite["code"],
                "cli": True,
            },
        )["token"]
        self.bob_id = self.req("whoami", token=self.bob)["id"]
        assert self.req("mission.list", token=self.bob) == []
        self.req("read", {"thread": self.secret}, self.bob, denied=True)
        group = self.req("group.create", {"name": "Developers"})["id"]
        self.req("group.add", {"group": group, "person": self.bob_id})
        self.req("member.add", {"mission": self.mid, "principal": group})
        assert len(self.req("mission.list", token=self.bob)) == 1
        service = self.req("account.service", {"name": "Release service"})
        self.req("member.add", {"mission": self.mid, "principal": service["id"]})
        assert self.req("whoami", token=service["token"])["name"] == "Release service"
        self.req("read", {"thread": self.secret}, service["token"], denied=True)
        token_id = self.req("token.list", token=service["token"])[0]["id"]
        self.req("token.revoke", {"id": token_id})
        self.req("whoami", token=service["token"], denied=True)
        assert (
            self.req("inbox.settings", {"settings": {"following": False}})["following"]
            is False
        )
        assert self.req("inbox.settings", token=self.bob)["following"] is True
        self.req("inbox.settings", {"settings": {"global": "no"}}, denied=True)
        self.req(
            "device.subscribe",
            {
                "subscription": {
                    "endpoint": "http://127.0.0.1/push",
                    "keys": {"auth": "x", "p256dh": "y"},
                }
            },
            denied=True,
        )
        print(
            "Passed: bootstrap, local invites, groups, mission visibility, service tokens",
            flush=True,
        )

    def oidc(self):
        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )
        opener.open(self.url + "/auth/oidc/start").read()

        def who():
            req = urllib.request.Request(
                self.url + "/api/whoami", b"{}", {"Content-Type": "application/json"}
            )
            return json.load(opener.open(req))

        user = who()
        assert user["name"] == "OIDC Teammate" and not user["owner"]
        cli_env = dict(self.env, RALLY_CONFIG_DIR=str(self.root / "oidc-profile"))
        process = subprocess.Popen(
            [BIN / "rally", "--json", "login", "--oidc", "--no-browser"],
            env=cli_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            assert "Open this URL" in process.stderr.readline()
            opener.open(process.stderr.readline().strip()).read()
            result, errors = process.communicate(timeout=10)
            assert process.returncode == 0, errors
            assert json.loads(result)["id"] == user["id"]
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
        print(
            "Passed: trusted issuer JIT provisioning, signed ID tokens and CLI PKCE callback",
            flush=True,
        )

    def conversations(self):
        self.agent = self.req(
            "agent.create",
            {
                "name": "engineer",
                "model": "gpt-6.2-luna",
                "instructions": "Record design decisions.",
            },
        )["id"]
        self.req(
            "post",
            {"thread": self.mid, "body": "HOLD", "mentions": [self.agent]},
            denied=True,
        )
        assert self.req("read", {"thread": self.mid})["messages"] == []
        result = self.req(
            "post",
            {
                "thread": self.mid,
                "body": "HOLD QUESTION",
                "mentions": [self.agent],
                "confirm": True,
                "key": "dispatch-1",
            },
        )
        rid = result["runs"][0]
        assert self.req(
            "post",
            {
                "thread": self.mid,
                "body": "HOLD QUESTION",
                "mentions": [self.agent],
                "key": "dispatch-1",
            },
        )["duplicate"]
        get = lambda: self.req("run.show", {"run": rid})
        question = wait(lambda: get()["questions"], "No queued question")[0]
        self.req("post", {"thread": self.mid, "body": "implicit steer"}, denied=True)
        self.req("run.steer", {"run": rid, "body": "PROBE alice"})
        wait(
            lambda: self.secret + ";" in get()["partial"],
            "Alice could not retrieve private context",
        )
        self.req(
            "post",
            {
                "thread": self.mid,
                "body": "An unaddressed observation during the active run",
            },
            self.bob,
        )
        self.req("run.steer", {"run": rid, "body": "PROBE bob"}, self.bob)
        wait(
            lambda: "Sources=;" in get()["partial"],
            "Bob steering did not narrow retrieval",
        )
        self.req(
            "run.steer",
            {"run": rid, "body": "PROBE explicit source", "refs": [self.secret]},
            self.bob,
            denied=True,
        )
        self.req(
            "post",
            {
                "thread": self.mid,
                "body": "PROBE explicit source",
                "refs": [self.secret],
                "mentions": [self.agent],
            },
        )
        wait(
            lambda: get()["partial"].count(self.secret + ";") >= 2,
            "Explicit cross-mission grant missing",
        )
        self.req(
            "run.answer",
            {
                "run": rid,
                "question": question["id"],
                "answer": {"scope": {"answers": ["Small"]}},
            },
            self.bob,
        )
        wait(
            lambda: "Question answered" in get()["partial"],
            "Answer did not reach harness",
        )
        self.req(
            "run.answer",
            {"run": rid, "question": question["id"], "answer": {}},
            denied=True,
        )
        self.req("run.steer", {"run": rid, "body": "FINISH"})
        wait(lambda: get()["status"] == "done", "Agent did not finish")
        assert self.req("inbox.list")[0]["reason"] == "done"
        side = self.req("read", {"thread": get()["working_thread"]})
        assert any(m["body"] == "PROBE bob" for m in side["messages"])
        self.req(
            "agent.update",
            {"agent": self.agent, "instructions": "Use the revised role."},
        )
        self.req(
            "agent.update",
            {"agent": self.agent, "instructions": "Invalid edit"},
            self.bob,
            denied=True,
        )
        rid2 = self.req(
            "post",
            {
                "thread": self.mid,
                "body": "HOLD after the first turn",
                "mentions": [self.agent],
            },
        )["runs"][0]
        get2 = lambda: self.req("run.show", {"run": rid2})
        wait(lambda: "Working" in get2()["partial"], "Second invocation missing")
        assert get()["session"] == get2()["session"]
        prompt = (
            self.root / "data/sessions" / get2()["session"] / "last-input.txt"
        ).read_text()
        assert "unaddressed observation" in prompt and "Use the revised role" in prompt
        self.stop()
        self.server()
        wait(lambda: get2()["status"] == "done", "Interrupted work did not resume")
        assert "Continued in the same native session." in get2()["partial"]
        output = self.cli("pull", self.mid)
        assert (
            "message:" in output
            and "token_digest" not in output
            and "embedding:" not in output
        )
        assert (
            self.req(
                "search",
                {"query": "privatebudgetmarker", "mission": self.secret},
                self.bob,
            )
            == []
        )
        late = self.req(
            "post", {"thread": self.mid, "body": "ASYNC_CARD", "mentions": [self.agent]}
        )["runs"][0]
        card = lambda: self.req("run.show", {"run": late})
        wait(
            lambda: card()["status"] == "done",
            "Async question did not finish its first turn",
        )
        q = card()["questions"][0]
        self.req(
            "run.answer",
            {"run": late, "question": q["id"], "answer": {"0": {"answers": ["Amber"]}}},
        )
        wait(
            lambda: any(
                m["body"] == "Applied the queued answer."
                for m in self.req("read", {"thread": self.mid})["messages"]
            ),
            "Late answer did not reopen the harness",
        )
        assert any(
            m["body"] == "Which colour?"
            for m in self.req("read", {"thread": card()["working_thread"]})["messages"]
        )
        print(
            "Passed: dispatch/steer, mixed authority and grants, queued questions, unseen history, restart, YAML",
            flush=True,
        )

    def git_and_skills(self):
        gitdir = self.root / "git"
        gitdir.mkdir()
        remote = gitdir / "project.git"
        self.git(gitdir, "init", "--bare", "--initial-branch=main", str(remote))
        dev = self.root / "developer"
        self.git(self.root, "clone", str(remote), str(dev))
        (dev / "README.md").write_text("Design decisions\n")
        (dev / "SKILL.md").write_text(
            "---\nname: reviewer\ndescription: Review work\n---\nCheck the diff.\n"
        )
        self.git(dev, "add", ".")
        self.git(dev, "commit", "--no-gpg-sign", "-m", "Initial")
        self.git(dev, "push", "-u", "origin", "main")
        url = "https://rally.invalid/project.git"
        repo = {"mission": self.mid, "name": "project", "remote": url, "branch": "main"}
        assert self.req("repo.add", repo)[0]["status"]["state"] == "current"
        hosted = self.root / "data/missions" / self.mid / "project"
        (dev / "upstream").write_text("upstream")
        self.git(dev, "add", ".")
        self.git(dev, "commit", "-m", "Upstream")
        self.git(dev, "push")
        (hosted / "local").write_text("local")
        self.git(hosted, "add", ".")
        self.git(hosted, "commit", "-m", "Local")
        assert self.req("repo.sync", repo)[0]["status"]["state"] == "diverged"
        (hosted / "more").write_text("work in progress")
        wip = self.req("repo.wip", dict(repo, summary="save review work", fork=True))[
            0
        ]["recovery"]
        assert wip.startswith("rally/wip/")
        self.git(dev, "fetch")
        self.git(dev, "merge", "--no-edit", "origin/" + wip)
        self.git(dev, "push")
        result = self.req("repo.sync", repo)[0]
        assert result["recovery"] is None and result["status"]["state"] == "current"
        assert self.git(hosted, "branch", "--show-current") == "main"
        assert not self.git(dev, "ls-remote", "--heads", "origin", wip)
        (hosted / "design.md").write_text("First draft")
        self.git(hosted, "add", "design.md")
        self.git(hosted, "commit", "-m", "Draft design")
        recovery = self.req("repo.wip", dict(repo, fork=True))[0]["recovery"]
        assert self.req("repo.wip", dict(repo, fork=True))[0]["recovery"] == recovery
        self.git(dev, "fetch")
        self.git(dev, "merge", "--squash", "origin/" + recovery)
        self.git(dev, "commit", "-m", "Publish design")
        self.git(dev, "push")
        assert self.req("repo.sync", repo)[0]["recovery"] == recovery
        returned = self.req("repo.return", repo)[0]
        assert returned["recovery"] is None and returned["status"]["state"] == "current"
        assert self.git(dev, "ls-remote", "--heads", "origin", recovery)
        sk = self.req(
            "skill.import", {"name": "reviewer", "remote": url, "ref": "main"}
        )["id"]
        self.req("skill.attach", {"skill": sk, "target": self.mid})
        (dev / "SKILL.md").write_text(
            "---\nname: reviewer\ndescription: Review work\n---\nCheck the accepted revision.\n"
        )
        self.git(dev, "add", "SKILL.md")
        self.git(dev, "commit", "-m", "Improve skill")
        self.git(dev, "push")
        wait(
            lambda: (
                "accepted revision" in self.req("skill.show", {"skill": sk})["body"]
            ),
            "Continuous import stalled",
            70,
        )
        self.cli(
            "config",
            "set",
            "--key",
            "directory",
            "--value",
            str(self.root / "local-missions"),
        )
        workspace = pathlib.Path(self.cli("open", self.mid, "--print-path"))
        assert pathlib.Path(self.cli("open", self.mid, "--print-path")) == workspace
        assert (workspace / "project/.agents/skills/reviewer/SKILL.md").is_file()
        assert (workspace / ".claude/skills/rally").is_symlink()
        self.cli("hooks", "install", "codex")
        self.cli("hooks", "install", "claude")
        event = {
            "cwd": str(workspace / "project"),
            "session_id": "test-local",
            "turn_id": "t-1",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Why do we tolerate duplicate events?",
        }
        self.cli("hooks", "capture", "codex", input=json.dumps(event))
        self.cli("hooks", "flush")
        event.update(
            hook_event_name="Stop",
            last_assistant_message="An idempotency key prevents repeated delivery from causing duplicate submissions.",
        )
        self.cli("hooks", "capture", "codex", input=json.dumps(event))
        self.cli("hooks", "flush")
        self.cli("hooks", "capture", "codex", input=json.dumps(event))
        self.cli("hooks", "flush")
        ext = next(
            t
            for t in self.req("thread.list", {"mission": self.mid})
            if t["kind"] == "external"
        )
        assert len(self.req("read", {"thread": ext["id"]})["messages"]) == 2
        transcript = self.root / "claude-transcript.jsonl"
        blocks = [
            {"type": "tool_use", "name": "unused"},
            {"type": "text", "text": "Keep the original reasoning with the work."},
        ]
        transcript.write_text(
            json.dumps({"type": "assistant", "message": {"content": blocks}}) + "\n"
        )
        claude = dict(
            event, session_id="claude-session", transcript_path=str(transcript)
        )
        del claude["last_assistant_message"]
        self.cli("hooks", "capture", "claude", input=json.dumps(claude))
        self.cli("hooks", "flush")
        found = self.req("search", {"query": "original reasoning", "mission": self.mid})
        assert any(
            x["source"] == "claude"
            and x["body"] == "Keep the original reasoning with the work."
            for x in found
        )
        self.cli("hooks", "uninstall", "claude")
        assert "absent" in self.cli("hooks", "status", "claude")
        event.update(cwd=str(self.root), session_id="unrelated")
        self.cli("hooks", "capture", "codex", input=json.dumps(event))
        hits = self.req("search", {"query": "idempotency", "mission": self.mid})
        assert any(h["thread"] == ext["id"] for h in hits)
        if self.env.get("RALLY_EMBEDDING_DIR"):
            wait(
                lambda: any(
                    h["thread"] == ext["id"]
                    for h in self.req(
                        "search",
                        {
                            "query": "retries and duplicated requests",
                            "mission": self.mid,
                        },
                    )
                ),
                "Semantic retrieval failed",
            )
        shutil.move(workspace, workspace.with_name("moved"))
        assert json.loads(self.cli("--json", "mission", "list", "--local")) == []
        self.req("repo.remove", repo)
        self.req(
            "repo.add",
            dict(repo, remote="https://rally.invalid/other.git"),
            denied=True,
        )
        print(
            "Passed: upstream divergence, WIP merge/cleanup, continuous Git skills, local hooks, deduplication, search",
            flush=True,
        )

    def routines(self):
        result = self.req(
            "routine.create",
            {
                "name": "Release Bot",
                "command": f"rally post --thread {self.mid} --mention {self.agent} --body 'Investigate the release'",
                "cron": "",
            },
        )
        assert self.req("routine.list", token=self.bob) == []
        self.req("routine.show", {"routine": result["id"]}, self.bob, denied=True)
        self.req(urllib.parse.urlsplit(result["webhook"]).path, {"event": "review"})
        show = lambda: self.req("routine.show", {"routine": result["id"]})
        wait(
            lambda: show()["jobs"] and show()["jobs"][0]["status"] == "done",
            "Webhook routine failed",
        )
        wait(
            lambda: any(
                m["author"] == "Release Bot"
                for m in self.req("read", {"thread": self.mid})["messages"]
            ),
            "Routine attribution lost",
        )
        timed = self.req(
            "routine.create",
            {"name": "Heartbeat", "command": "echo heartbeat", "cron": "*/2 * * * * *"},
        )["id"]
        wait(
            lambda: any(
                j["status"] == "done"
                for j in self.req("routine.show", {"routine": timed})["jobs"]
            ),
            "Cron never ran",
        )
        self.req("routine.disable", {"routine": timed})
        self.req("routine.transfer", {"routine": timed, "person": self.bob_id})
        assert self.req("routine.list", token=self.bob)[0]["id"] == timed
        waiting = self.req(
            "routine.create",
            {
                "name": "Wait for review",
                "command": f"trap 'echo terminated; exit 0' TERM; rally post --thread {self.mid} --mention {self.agent} --body HOLD; while :; do sleep 1; done",
            },
        )
        self.req("routine.trigger", {"routine": waiting["id"]})

        def active_run():
            return next(
                (
                    r["id"]
                    for r in self.req("run.list", {"mission": self.mid})
                    if r["status"] == "working"
                    and "Working." in self.req("run.show", {"run": r["id"]})["partial"]
                ),
                None,
            )

        detached = wait(active_run, "Routine did not dispatch a resumable invocation")
        self.stop()
        self.server()
        job = self.req("routine.show", {"routine": waiting["id"]})["jobs"][0]
        assert job["status"] == "aborted" and "terminated" in job["output"], job
        wait(
            lambda: self.req("run.show", {"run": detached})["status"] == "done",
            "Detached dispatch failed to resume",
        )
        print(
            "Passed: cron, webhook, ownership and independently dispatched mission work",
            flush=True,
        )


def main():
    admin = os.environ["RALLY_TEST_ADMIN_URL"]
    name = "rally_test_" + secrets.token_hex(7)
    parsed = urllib.parse.urlsplit(admin)
    db = urllib.parse.urlunsplit(parsed._replace(path="/" + name))
    subprocess.run(
        ["psql", admin, "-X", "-v", "ON_ERROR_STOP=1", "-c", f"CREATE DATABASE {name}"],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    try:
        with tempfile.TemporaryDirectory(prefix="rally-suite-") as tmp:
            suite = Suite(pathlib.Path(tmp), db)
            try:
                suite.server()
                suite.auth()
                suite.oidc()
                suite.conversations()
                suite.git_and_skills()
                suite.routines()
                if os.environ.get("RALLY_TEST_BROWSER") == "1":
                    subprocess.run(
                        ["bun", "x", "playwright", "test"],
                        cwd=ROOT / "web",
                        env=dict(
                            suite.env,
                            RALLY_TEST_EMAIL=EMAIL,
                            RALLY_TEST_PASSWORD=PASSWORD,
                            RALLY_TEST_HARNESS="1",
                        ),
                        check=True,
                    )
            except Exception:
                print((suite.root / "server.log").read_text()[-3000:])
                raise
            finally:
                suite.stop()
                suite.issuer.close()
                suite.log.close()
    finally:
        subprocess.run(
            [
                "psql",
                admin,
                "-X",
                "-v",
                "ON_ERROR_STOP=1",
                "-c",
                f"DROP DATABASE {name} WITH (FORCE)",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )


if __name__ == "__main__":
    main()
