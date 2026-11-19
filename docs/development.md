# Develop and verify Rally

The server entry point is `src/bin/rally-server.rs`. The API, database schema, Git reconciliation, search, workers and scheduler live in `src/server/`. The CLI uses the same HTTP endpoints from `src/client/`. The client is a React app built by Bun in `web/`.

## Build and run the test suite

Install stable Rust, Bun, Git, OpenSSL, Python 3, a PostgreSQL client and a PostgreSQL instance where the test user may create databases. Linux builds use pkg-config, libssl and libcurl development packages.

```sh
cargo fmt --all --check
cargo test --locked
cargo build --locked --bins
(cd web && bun install --frozen-lockfile && bun x tsc --noEmit && bun run build && bun x playwright install chromium)
sh scripts/fetch-embeddings.sh "$PWD/.rally-test/embeddings"
export RALLY_EMBEDDING_DIR="$PWD/.rally-test/embeddings"
export RALLY_TEST_ADMIN_URL=postgres://postgres@localhost/postgres
RALLY_TEST_BROWSER=1 python3 tests/integration.py
```

Supply the appropriate credentials for your test database. With system Chromium, set `CHROMIUM_PATH` to the executable. Browser screenshots and failed traces are under `web/test-results/`. The integration suite creates a randomly named database and temporary workspace, runs the real binaries, then drops that database. It exercises signed OIDC, local membership, harness messages and recovery, Git divergence, two contributor identities on a disposable HTTPS forge, delegated push and takeover, silent routine dispatch, skill sync and reconnection, external hooks, search and cron/webhook routines. A deterministic protocol peer lets CI test dispatch and steering without provider credentials.

For local development serve `web/dist` with the server after building the app. The server reads `DATABASE_URL`, `RALLY_DATA`, `RALLY_BOOTSTRAP_SECRET` and optionally `RALLY_WEB_DIR`. `RALLY_ENGINE=local` executes commands directly as the server's OS user. Reserve that setting for development fixtures on a trusted machine. The deployment uses rootless Podman.

## Exercise real worker containers

Run `python3 tests/compose_setup.py` on a host with podman-compose and PyYAML to check fresh setup, repair of an older `.env`, and the resolved Codex bind mount in a temporary checkout.

Use a disposable installation with a separate Compose project, socket owner or dedicated Podman user, data directory, credentials, network, published port and runtime image tag. Run the commands below from the repository root. Set `RALLY_CODEX_CONFIG` to the absolute path to `tests/provider.toml` for the offline provider. Supply your disposable configuration as `test.env` and keep it out of version control. The provider override mounts an offline Responses API fixture into a separate container on the test network. It requires no model account.

```sh
export RALLY_ADAPTER=acp
podman-compose --env-file test.env -p rally_test -f compose.yml -f tests/compose-provider.yml up -d --build
export RALLY_URL=http://127.0.0.1:18879
export RALLY_TEST_WORKSPACES=/absolute/path/to/test-data/missions
export RALLY_TEST_EMAIL=operator@example.test
export RALLY_TEST_PASSWORD=your-disposable-password
python3 tests/hosted_smoke.py
```

Set `RALLY_LISTEN=127.0.0.1:18879` in the disposable config for the example port. Bootstrap the fixture's organisation owner first. The hosted smoke test dispatches two turns to a real Codex harness, runs a shell command inside its container and checks the resulting file on the host. Set `RALLY_TEST_GIT_ORIGIN` for the smoke test if you have started the disposable HTTPS forge from `tests/git_forge.py` on the test network and configured its CA on the server. The offline provider will also ask Codex to fetch and push the fixture repository. Recreate the server with `RALLY_ADAPTER=codex` to exercise its native protocol. For the routine round trip, set `RALLY_TOKEN` to a token from this test organisation and run `python3 tests/compose_smoke.py`. The routine verifies the engine socket is absent in its container and uses the mounted CLI to post to the test mission. Shut down the specific Compose project with the same env file and file list.

## CI and releases

GitHub Actions runs Rust and frontend builds and the PostgreSQL/Chromium integration suite. A pushed `v*` tag builds Linux x86-64 and ARM64 bundles (server, CLI, web app, worker context and docs) and a macOS ARM64 CLI archive. The release workflow uploads archives and SHA-256 manifests to GitHub Releases. To test Linux packaging on a workstation, run:

```sh
cargo build --locked --release --bins
(cd web && bun run build)
sh scripts/package-release.sh v0.1.0 linux-x86_64
(cd artifacts && sha256sum -c *.sha256)
```
