# Install a self-hosted server

## Rootless Podman Compose

Use a dedicated Linux user for Rally with rootless Podman, podman-compose, OpenSSL, and a systemd user session. Clone the repository, start that user's socket, and run:

```sh
systemctl --user enable --now podman.socket
sh scripts/setup-compose.sh
podman-compose -p rally build server
podman-compose -p rally run --rm --no-deps server login
podman-compose -p rally up -d --build
```

The setup script writes a private `.env` with generated database and bootstrap credentials. It also records the absolute host data path and socket location. Open `http://localhost:8787`, create the organisation owner using the bootstrap secret, then invite other local users through Settings. The server runs one instance against its database. Check readiness with `curl --fail http://localhost:8787/health`.

For access from other devices, point your HTTPS reverse proxy at the loopback listener. Set `RALLY_URL` to the public HTTPS origin before starting. Forward the event stream at `/events` with response buffering disabled. HTTPS enables secure cookies, PWA installation and push. The development loopback origin also works for local testing.

| Setting | Purpose |
| --- | --- |
| `RALLY_HOST_DATA` | Absolute data directory as seen by the Podman service. The server sees this directory at `/var/lib/rally`. Both must refer to the same files. |
| `RALLY_PODMAN_SOCKET` | Dedicated rootless socket mounted into the server. Access to it gives the server control of this user's containers and file mounts. Work containers receive only the configured mounts. |
| `RALLY_NETWORK` | Network shared by the server and workers. Give each installation its own network, data path, runtime image tag, database and Compose project name. |
| `RALLY_RUNTIME_DIR` | Operator-maintained build context containing the worker Containerfile. Defaults to `./runtime`. |
| `DATABASE_URL` | An existing PostgreSQL database. With an external database run `podman-compose -p rally up -d --build --no-deps server`. The database role needs permission to create and update tables in this database. The Compose file still requires its DB password variable for interpolation. |
| `RALLY_LISTEN` | Host interface and port to publish, default `127.0.0.1:8787`. |

The first start builds the shared runtime image. Public package registries and the pinned embedding model download must be reachable during image builds. Worker networking must allow access to Rally, relevant Git remotes and your configured model provider. Configure the operator's Git credentials separately for private server-side skill imports.

## Codex configuration and login

Every installation supplies a Codex TOML path. With the server binary use `--codex-config /path/to/config.toml`. In Compose set `RALLY_CODEX_CONFIG` in `.env` to the absolute host path. The setup script selects the supplied example, `examples/config.toml`. Edit it or select another file and recreate the server to apply changes. Existing checkouts with an older `.env` can add this setting by hand.

Choose one provider identity for the installation.

| Method | Startup behaviour |
| --- | --- |
| Rally login | Run the login command shown above. Rally runs Codex device login in a disposable worker container and saves the resulting file privately to `$RALLY_DATA/codex/auth.json`. An interactive binary startup offers this login automatically when the default file is missing. Unattended startup reports the missing login and exits. |
| Existing Codex login | Pass `--auth /path/to/auth.json`, or set `RALLY_CODEX_AUTH` to a path **visible to the server**. Rally reads and refreshes that file. In Compose, a file under `$RALLY_HOST_DATA/provider/auth.json` is visible at `/var/lib/rally/provider/auth.json`. The parent directory must be writable for atomic replacement. |
| Custom provider | Set `--bypass-auth` or `RALLY_BYPASS_AUTH=true`. This takes precedence over both credential files. Select the provider in the TOML and supply any required provider variables through `RALLY_CODEX_ENV_FILE`, a dotenv file visible to the server. For example, place it under `$RALLY_HOST_DATA/provider/gateway.env` and use `/var/lib/rally/provider/gateway.env` in Compose. |

For binary deployments, run `rally-server --codex-config /path/to/config.toml login` with the same data directory, runtime image, build context and Podman settings as the server. Explicit login always writes the Rally default. Use `login status` to check the selected source. Rally supplies each new Codex process with its configuration and the selected identity. Native ChatGPT sessions receive access tokens over the harness protocol. Rally coordinates refreshes centrally. A session's native history stays in its own writable directory under `$RALLY_DATA/sessions`.

If the provider requires interactive reauthentication, Settings shows the error under **Models**. Connect to the host and repeat the login command for the Rally-managed identity, or renew the selected external file when using `--auth`. Keep login files and provider dotenv files outside the worker image.

## OIDC and push

Register `https://YOUR-RALLY/auth/oidc/callback` as an exact redirect URI at your identity provider. Configure `RALLY_OIDC_ISSUER`, `RALLY_OIDC_CLIENT_ID`, and, for a confidential client, `RALLY_OIDC_CLIENT_SECRET`. A successful trusted-issuer login provisions a member keyed by issuer and subject. Bootstrap the first owner using the setup screen. Add the new OIDC member to missions or groups, or grant access to all missions by choosing Admin for that account in Settings. The browser and CLI use the same provider registration.

Generate a P-256 VAPID pair with `bunx web-push generate-vapid-keys`. Store the two URL-safe base64 values in `RALLY_VAPID_PUBLIC` and `RALLY_VAPID_PRIVATE`. Keep the private value and database credentials in the operator's configuration. Set `RALLY_VAPID_CONTACT` to a reachable mailto address. Each device registers from Settings. Browsers use their own push service, so the server needs outbound HTTPS to that service.

## Binary deployment and data

On Linux, unpack the release, put both binaries alongside each other on PATH, and run `rally-server --codex-config /path/to/config.toml` with an existing `DATABASE_URL`, `RALLY_BOOTSTRAP_SECRET`, public `RALLY_URL`, absolute `RALLY_DATA` and `RALLY_HOST_DATA`, and the network and Podman socket address (`RALLY_PODMAN_URL=unix:///…`). Set `RALLY_WEB_DIR` to the unpacked `web/dist`, `RALLY_BUILD_DIR` to the unpacked `runtime`, and `RALLY_WORKER_URL` to a URL reachable from worker containers. Install Git and the Podman client on the server. Use `scripts/fetch-embeddings.sh /path/to/model` and set `RALLY_EMBEDDING_DIR` to that directory to enable local semantic search. Without that variable the server serves PostgreSQL lexical search.

For a consistent backup, gracefully stop the server, take a PostgreSQL dump including its schema, and copy the entire host data directory. Include `keys/secrets.key` from the data directory. The encrypted secrets in PostgreSQL require this key. Protect the backup, including the shared Codex login, and save operator configuration and worker build context separately. The data directory includes unpushed work and the native session state required to resume work. Restore the database and matching directory together, correct the absolute mount paths, then start Rally. Back up Git remotes separately.

For an upgrade, back up, update the checkout and recreate the server with `podman-compose -p rally up -d --build --force-recreate --no-deps server`. Startup applies the bundled schema and rebuilds the shared runtime. A graceful shutdown sends termination to active workers and records interrupted mission runs for the next startup.
