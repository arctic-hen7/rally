"""A disposable HTTPS Git smart HTTP server backed by real bare repositories."""

import base64
import http.server
import os
import ssl
import subprocess
import threading
from urllib.parse import urlsplit


class Forge:
    def __init__(self, root, address=("127.0.0.1", 0)):
        self.calls = []
        self.passwords = {"alice": "alice-fixture", "bob": "bob-fixture"}
        self.ca = root / "forge-ca.pem"
        key = root / "forge-key.pem"
        cert = root / "forge-cert.pem"
        csr = root / "forge.csr"
        ca_key = root / "forge-ca.key"
        commands = [
            [
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-subj",
                "/CN=Fixture CA",
                "-keyout",
                ca_key,
                "-out",
                self.ca,
                "-days",
                "1",
            ],
            [
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-subj",
                "/CN=localhost",
                "-keyout",
                key,
                "-out",
                csr,
                "-addext",
                "subjectAltName=IP:127.0.0.1,DNS:forge",
            ],
            [
                "x509",
                "-req",
                "-in",
                csr,
                "-CA",
                self.ca,
                "-CAkey",
                ca_key,
                "-CAcreateserial",
                "-out",
                cert,
                "-days",
                "1",
                "-copy_extensions",
                "copy",
            ],
        ]
        for command in [] if cert.is_file() else commands:
            subprocess.run(
                ["openssl", *command],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
            )
        forge = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def serve(self):
                user = ""
                for name, password in forge.passwords.items():
                    basic = base64.b64encode(f"{name}:{password}".encode()).decode()
                    if self.headers.get("Authorization") == "Basic " + basic:
                        user = name
                url = urlsplit(self.path)
                if not user and not url.path.startswith("/public.git/"):
                    self.send_error(401)
                    return
                forge.calls.append((user, url.path, url.query))
                data = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                env = dict(
                    os.environ,
                    GIT_PROJECT_ROOT=str(root / "git"),
                    GIT_HTTP_EXPORT_ALL="1",
                    PATH_INFO=url.path,
                    QUERY_STRING=url.query,
                    REQUEST_METHOD=self.command,
                    CONTENT_TYPE=self.headers.get("Content-Type", ""),
                    CONTENT_LENGTH=str(len(data)),
                    REMOTE_USER=user,
                    HTTP_GIT_PROTOCOL=self.headers.get("Git-Protocol", ""),
                )
                response = subprocess.check_output(
                    ["git", "http-backend"], env=env, input=data
                )
                headers, body = response.split(b"\r\n\r\n", 1)
                pairs = dict(h.decode().split(": ", 1) for h in headers.split(b"\r\n"))
                self.send_response(int(pairs.pop("Status", "200 OK").split()[0]))
                for h, v in pairs.items():
                    self.send_header(h, v)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            do_GET = serve
            do_POST = serve

        self.server = http.server.ThreadingHTTPServer(address, Handler)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cert, key)
        self.server.socket = tls.wrap_socket(self.server.socket, server_side=True)
        self.origin = f"https://127.0.0.1:{self.server.server_port}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


if __name__ == "__main__":
    import pathlib
    import sys

    fixture = Forge(pathlib.Path(sys.argv[1]), ("0.0.0.0", 8443))
    threading.Event().wait()
