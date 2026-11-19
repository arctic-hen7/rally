# Rally

Rally is a self-hosted workspace for humans and agents. A mission groups a discrete task, its Git branches, and the conversations that explain the work. Start a new task, search for an earlier decision, and bring the original discussion into a fresh agent session.

The Rust server stores mission state in PostgreSQL. The React PWA, Rust CLI, external session hooks, hosted agents, and scheduled or webhook-driven routines all use the same API.

## Start a server

On Linux with rootless Podman, podman-compose, and OpenSSL installed, prepare the host configuration from the repository root.

```sh
systemctl --user start podman.socket
sh scripts/setup-compose.sh
```

Setup copies the bundled Codex example to `.rally-data/config.toml` and records its absolute host path as `RALLY_CODEX_CONFIG` in `.env`. You can edit that copy or point the setting at your own TOML file. If you already have a Rally `.env`, rerun setup to add the missing setting while keeping your credentials.

For a shared Codex account, build the server, sign in, then start Rally.

```sh
podman-compose -p rally build server
podman-compose -p rally run --rm --no-deps server login
podman-compose -p rally up -d --build
```

Open **http://localhost:8787** and create the first owner using the bootstrap secret in `.env`. The initial build downloads the pinned CPU embedding model and installs Codex and its ACP adapter. The login command uses a device code to connect one Codex account for hosted agents. Custom providers can use the bypass mode described in the installation guide. Create a topic and a mission to begin.

## Use the CLI

```sh
cargo install --path . --bin rally --locked
rally login --url http://localhost:8787 --email you@example.org
rally config set --key directory --value "$HOME/rally"
eval "$(rally shell-init bash)"
rally open '#dev-1'
rally search 'why did we choose this approach?'
```

Release archives include the CLI. Linux archives also include the server and compiled web app.

- [Personal Git connections and repository handover](docs/git-connections.md)
- [Installation, credentials, backups, and upgrades](docs/installation.md)
- [Missions, Git recovery, access, and notifications](docs/missions.md)
- [CLI, local workspaces, external sessions, and exports](docs/cli.md)
- [Agents, skills, containers, and routines](docs/agents.md)
- [Development and integration tests](docs/development.md)

Rally is dual-licensed under the [MIT licence](LICENSE-MIT) or [Apache Licence 2.0](LICENSE-APACHE), at your option.
