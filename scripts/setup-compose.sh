#!/bin/sh
set -eu
umask 077
if [ -f .env ] && grep -Eq '^[[:space:]]*(export[[:space:]]+)?RALLY_CODEX_CONFIG[[:space:]]*=' .env; then
    echo "Using the RALLY_CODEX_CONFIG setting already in .env. It must point to a readable TOML file on the host."
    exit 0
fi
if [ ! -f examples/config.toml ]; then
    echo "Run this script from the Rally checkout containing examples/config.toml." >&2
    exit 1
fi
root=$(pwd -P)
config=$root/.rally-data/config.toml
mkdir -p .rally-data
if [ ! -e "$config" ]; then
    (set -C; cat examples/config.toml > "$config")
fi
if [ ! -f "$config" ] || [ ! -r "$config" ]; then
    echo "Expected a readable Codex TOML file at $config." >&2
    exit 1
fi
if [ -f .env ]; then
    printf '\nRALLY_CODEX_CONFIG=%s\n' "$config" >> .env
    echo "Added RALLY_CODEX_CONFIG to .env. Edit $config to configure Codex."
    exit 0
fi
socket=${RALLY_PODMAN_SOCKET:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock}
db_password=$(openssl rand -hex 24)
bootstrap=$(openssl rand -hex 24)
set -C
cat >.env <<EOF
RALLY_CODEX_CONFIG=$config
RALLY_DB_PASSWORD=$db_password
RALLY_BOOTSTRAP_SECRET=$bootstrap
RALLY_HOST_DATA=$root/.rally-data
RALLY_PODMAN_SOCKET=$socket
RALLY_URL=http://localhost:8787
RALLY_NETWORK=rally_default
EOF
echo "Created .env. Edit $config to configure Codex. The first owner signs up using RALLY_BOOTSTRAP_SECRET from .env."
