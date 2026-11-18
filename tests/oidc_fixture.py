"""Minimal local issuer with real RSA signatures, state, nonce and PKCE."""

import base64
import hashlib
import http.server
import json
import secrets
import subprocess
import threading
import time
import urllib.parse


def encode(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


class Issuer:
    def __init__(self, root):
        key = root / "issuer.pem"
        subprocess.run(
            [
                "openssl",
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-out",
                key,
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        modulus = (
            subprocess.check_output(
                ["openssl", "rsa", "-in", key, "-modulus", "-noout"], text=True
            )
            .strip()
            .split("=")[1]
        )
        jwk = {
            "kty": "RSA",
            "alg": "RS256",
            "kid": "integration",
            "e": "AQAB",
            "n": encode(bytes.fromhex(modulus)),
        }
        codes = {}

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def send_json(self, data, status=200):
                content = json.dumps(data).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(content)))
                self.end_headers()
                self.wfile.write(content)

            def do_GET(self):
                parsed = urllib.parse.urlsplit(self.path)
                if parsed.path.endswith("openid-configuration"):
                    self.send_json(
                        {
                            "issuer": issuer,
                            "authorization_endpoint": issuer + "/authorize",
                            "token_endpoint": issuer + "/token",
                            "jwks_uri": issuer + "/keys",
                        }
                    )
                elif parsed.path == "/keys":
                    self.send_json({"keys": [jwk]})
                elif parsed.path == "/authorize":
                    query = dict(urllib.parse.parse_qsl(parsed.query))
                    code = secrets.token_hex(16)
                    assert query["code_challenge_method"] == "S256"
                    codes[code] = query
                    self.send_response(302)
                    self.send_header(
                        "Location",
                        query["redirect_uri"]
                        + "?"
                        + urllib.parse.urlencode(
                            {"code": code, "state": query["state"]}
                        ),
                    )
                    self.end_headers()
                else:
                    self.send_json({}, 404)

            def do_POST(self):
                fields = dict(
                    urllib.parse.parse_qsl(
                        self.rfile.read(int(self.headers["content-length"])).decode()
                    )
                )
                original = codes.pop(fields.get("code", ""), None)
                if not original or original["code_challenge"] != encode(
                    hashlib.sha256(fields["code_verifier"].encode()).digest()
                ):
                    self.send_json({"error": "invalid_grant"}, 400)
                    return
                claims = {
                    "sub": "subject-1",
                    "name": "OIDC Teammate",
                    "iss": issuer,
                    "aud": "rally",
                    "nonce": original["nonce"],
                    "exp": int(time.time()) + 120,
                }
                data = (
                    encode(json.dumps({"alg": "RS256", "kid": "integration"}).encode())
                    + "."
                    + encode(json.dumps(claims).encode())
                )
                signature = subprocess.check_output(
                    ["openssl", "dgst", "-sha256", "-sign", key], input=data.encode()
                )
                self.send_json(
                    {"id_token": data + "." + encode(signature), "token_type": "Bearer"}
                )

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        issuer = self.url = "http://127.0.0.1:" + str(self.server.server_port)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
