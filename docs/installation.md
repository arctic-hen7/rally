# Install a self-hosted server

## Rootless Podman Compose

Use a dedicated Linux user for Rally with rootless Podman, podman-compose, OpenSSL, and a systemd user session. Clone the repository, start that user's socket, and run:

```sh
systemctl --user enable --now podman.socket
sh scripts/setup-compose.sh
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

## OIDC and push

Register `https://YOUR-RALLY/auth/oidc/callback` as an exact redirect URI at your identity provider. Configure `RALLY_OIDC_ISSUER`, `RALLY_OIDC_CLIENT_ID`, and, for a confidential client, `RALLY_OIDC_CLIENT_SECRET`. A successful trusted-issuer login provisions a member keyed by issuer and subject. Bootstrap the first owner using the setup screen. Add the new OIDC member to missions or groups, or grant access to all missions by choosing Admin for that account in Settings. The browser and CLI use the same provider registration.

Generate a P-256 VAPID pair with `bunx web-push generate-vapid-keys`. Store the two URL-safe base64 values in `RALLY_VAPID_PUBLIC` and `RALLY_VAPID_PRIVATE`. Keep the private value and database credentials in the operator's configuration. Set `RALLY_VAPID_CONTACT` to a reachable mailto address. Each device registers from Settings. Browsers use their own push service, so the server needs outbound HTTPS to that service.

## Binary deployment and data

On Linux, unpack the release, put both binaries alongside each other on PATH, and run `rally-server` with an existing `DATABASE_URL`, `RALLY_BOOTSTRAP_SECRET`, public `RALLY_URL`, absolute `RALLY_DATA` and `RALLY_HOST_DATA`, and the network and Podman socket address (`RALLY_PODMAN_URL=unix:///…`). Set `RALLY_WEB_DIR` to the unpacked `web/dist`, `RALLY_BUILD_DIR` to the unpacked `runtime`, and `RALLY_WORKER_URL` to a URL reachable from worker containers. Install Git and the Podman client on the server. Use `scripts/fetch-embeddings.sh /path/to/model` and set `RALLY_EMBEDDING_DIR` to that directory to enable local semantic search. Without that variable the server serves PostgreSQL lexical search.

For a consistent backup, gracefully stop the server, take a PostgreSQL dump including its schema, and copy the entire host data directory. Save operator configuration, private keys and worker build context separately. The data directory includes unpushed work and the native session state required to resume work. Restore the database and matching directory together, correct the absolute mount paths, then start Rally. Back up Git remotes separately.

For an upgrade, back up, update the checkout and recreate the server with `podman-compose -p rally up -d --build --force-recreate --no-deps server`. Startup applies the bundled schema and rebuilds the shared runtime. A graceful shutdown sends termination to active workers and records interrupted mission runs for the next startup.
