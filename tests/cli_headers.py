"""Exercise the CLI against two origins using a disposable gateway and profile."""

import json
import os
import pathlib
import pty
import subprocess
import sys
import tempfile
import threading
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


@contextmanager
def endpoint():
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            requests.append((self.path, self.headers))
            if self.server.redirect:
                self.send_response(307)
                self.send_header("Location", self.server.redirect)
                self.end_headers()
                return
            if self.server.protected and (
                self.headers.get("X-Employee") != "sample"
                or self.headers.get("X-Gateway") != "credential"
            ):
                self.send_error(403)
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            polls = sum(path == "/api/run.show" for path, _ in requests)
            status = "working" if polls == 1 else "done"
            self.wfile.write(json.dumps({"token": "rally-session", "status": status}).encode())

        def log_message(self, *args):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        server.redirect = None
        server.protected = True
        server.url = f"http://127.0.0.1:{server.server_address[1]}"
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server, requests
        finally:
            server.shutdown()
            thread.join()


def contract(binary):
    with (
        tempfile.TemporaryDirectory() as tmp,
        endpoint() as (gateway, requests),
        endpoint() as (other, other_requests),
    ):
        profile = pathlib.Path(tmp)
        config = profile / "config.json"
        calls = profile / "calls"
        ready = profile / "ready"
        helper = profile / "helper.py"
        helper.write_text(
            "from pathlib import Path\n"
            "import sys\n"
            "with Path('calls').open('a') as log: print(sys.argv[1], file=log)\n"
            "if sys.argv[1] == 'login': Path('ready').touch()\n"
            "elif Path('ready').exists(): print('credential')\n"
            "else: sys.exit(1)\n"
        )
        command = [sys.executable, str(helper)]
        source = {"command": [*command, "lookup"], "login_command": [*command, "login"]}
        settings = {
            "url": gateway.url,
            "http_headers": {
                gateway.url: {"X-Employee": {"env": "TEST_EMPLOYEE"}, "X-Gateway": source}
            },
        }
        config.write_text(json.dumps(settings))
        env = dict(os.environ, RALLY_CONFIG_DIR=tmp, TEST_EMPLOYEE="sample")
        env.pop("RALLY_TOKEN", None)
        env.pop("RALLY_URL", None)
        other.protected = False

        def cli(*args, ok=True, stdin=subprocess.DEVNULL, **extra):
            proc = subprocess.run(
                [binary, *args], env=env | extra, stdin=stdin,
                capture_output=True, text=True, timeout=5,
            )
            assert (proc.returncode == 0) == ok, (args, proc.stderr)
            return proc

        cli("shell-init", "bash")
        assert not calls.exists()
        cli("login", "--token", "example", ok=False)
        assert calls.read_text().splitlines() == ["lookup"] and not requests
        master, slave = pty.openpty()
        try:
            cli("login", "--email", "qa@example.test", "--password", "qa", stdin=slave)
        finally:
            os.close(master)
            os.close(slave)
        assert calls.read_text().splitlines() == ["lookup", "lookup", "login", "lookup"]
        assert [p for p, _ in requests] == ["/auth/login", "/api/whoami"]
        assert requests[-1][1]["Authorization"] == "Bearer rally-session"

        outbox = profile / "outbox"
        outbox.mkdir()
        for i in range(3):
            (outbox / f"{i}.json").write_text(json.dumps({"server": gateway.url}))
        ready.unlink()
        cli("hooks", "flush", ok=False)
        assert len(list(outbox.glob("*.json"))) == 3 and not ready.exists()
        ready.touch()
        calls.write_text("")
        cli("hooks", "flush")
        assert calls.read_text().splitlines() == ["lookup"] and not list(outbox.glob("*.json"))

        cli("whoami", RALLY_URL=other.url)
        cli(
            "login", "--url", other.url, "--email", "qa@example.test",
            "--password", "qa", RALLY_TOKEN="work-session",
        )
        for _, headers in other_requests[:2]:
            assert not any(headers.get(h) for h in ("X-Employee", "X-Gateway", "Authorization"))
        assert calls.read_text().splitlines() == ["lookup"]

        settings["token"] = "rally-session"
        source["refresh_interval_ms"] = 50
        config.write_text(json.dumps(settings))
        calls.write_text("")
        cli("run", "follow", "--run", "test")
        assert calls.read_text().splitlines() == ["lookup", "lookup"]

        gateway.redirect = other.url + "/redirect-target"
        count = len(other_requests)
        assert "redirect" in cli("whoami", ok=False).stderr
        assert len(other_requests) == count
        gateway.redirect = None

        child_script = "import time, pathlib; time.sleep(1); pathlib.Path('survived').touch()"
        parent_script = (
            "import subprocess, sys; "
            f"subprocess.run([sys.executable, '-c', {child_script!r}])"
        )
        source.update(command=[sys.executable, "-c", parent_script], timeout_ms=250)
        config.write_text(json.dumps(settings))
        calls.write_text("")
        master, slave = pty.openpty()
        try:
            assert "timed out" in cli("login", "--token", "test", stdin=slave, ok=False).stderr
        finally:
            os.close(master)
            os.close(slave)
        time.sleep(1)
        assert calls.read_text() == "" and not (profile / "survived").exists()

        del source["login_command"]
        for script, error in [
            ("print()", "Empty"),
            ("print('first\\nsecond')", "single HTTP header"),
            ("print('x' * 100000)", "exceeds"),
        ]:
            source.update(command=[sys.executable, "-c", script], timeout_ms=150)
            config.write_text(json.dumps(settings))
            assert error in cli("whoami", ok=False).stderr
        source.update(env="TEST_EMPLOYEE")
        config.write_text(json.dumps(settings))
        assert "Invalid settings" in cli("whoami", ok=False).stderr
        settings["http_headers"][gateway.url] = {"Authorization": {"env": "TEST_EMPLOYEE"}}
        config.write_text(json.dumps(settings))
        assert "Rally manages" in cli("whoami", ok=False).stderr
        print("Passed CLI gateway headers, login, reuse, refresh, hook recovery, origin isolation and timeouts")


if __name__ == "__main__":
    contract(pathlib.Path(__file__).resolve().parents[1] / "target/debug/rally")
