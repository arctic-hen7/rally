# Rally

Rally is a self-hosted workspace for humans and agents. A mission groups a discrete task, its Git branches, and the conversations that explain the work. Start a new task, search for an earlier decision, and bring the original discussion into a fresh agent session.

The Rust server stores mission state in PostgreSQL. The React PWA, Rust CLI, external session hooks, hosted agents, and scheduled or webhook-driven routines all use the same API.

## Start a server

Use a Linux AMD64 or ARM64 host with rootless Podman, podman-compose, OpenSSL and a systemd user session. Download the deployment bundle from [GitHub Releases](https://github.com/arctic-hen7/rally/releases).

```sh
curl -fLO https://github.com/arctic-hen7/rally/releases/latest/download/rally-compose.tar.gz
tar -xzf rally-compose.tar.gz
cd rally
systemctl --user enable --now podman.socket
sh scripts/setup-compose.sh
podman-compose -p rally run --rm --no-deps server login
podman-compose -p rally up -d
```

Follow the Codex device-login prompt. Open **http://localhost:8787** and use `RALLY_BOOTSTRAP_SECRET` from the generated `.env` to create your owner account. The bundle selects matching server and worker images. Setup creates your editable Codex configuration at `.rally-data/config.toml`.

The deployment template lives in [examples/compose.yml](examples/compose.yml). For a custom model provider, HTTPS access, or updates, see the [installation guide](docs/installation.md). Contributors can build and test unreleased changes with the [source checkout](docs/development.md).

## Use the CLI

Download the binary archive for your platform from the same release and put `bin/rally` on your PATH.

```sh
rally login --url http://localhost:8787 --email you@example.org
rally config set --key directory --value "$HOME/rally"
eval "$(rally shell-init bash)"
rally open '#dev-1'
rally search 'why did we choose this approach?'
```

- [Personal Git connections and repository handover](docs/git-connections.md)
- [Installation, credentials, backups, and upgrades](docs/installation.md)
- [Missions, Git recovery, access, and notifications](docs/missions.md)
- [CLI, local workspaces, external sessions, and exports](docs/cli.md)
- [Agents, skills, containers, and routines](docs/agents.md)
- [Development and integration tests](docs/development.md)

Rally is dual-licensed under the [MIT licence](LICENSE-MIT) or [Apache Licence 2.0](LICENSE-APACHE), at your option.
