#!/bin/sh
set -eu
umask 077
if [ -e .env ]; then
    echo "An .env file already exists. Edit it to change this installation." >&2
    exit 1
fi
mkdir -p .rally-data
socket=${RALLY_PODMAN_SOCKET:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock}
db_password=$(openssl rand -hex 24)
bootstrap=$(openssl rand -hex 24)
cat >.env <<EOF
RALLY_CODEX_CONFIG=$(pwd -P)/examples/config.toml
RALLY_DB_PASSWORD=$db_password
RALLY_BOOTSTRAP_SECRET=$bootstrap
RALLY_HOST_DATA=$(pwd -P)/.rally-data
RALLY_PODMAN_SOCKET=$socket
RALLY_URL=http://localhost:8787
RALLY_NETWORK=rally_default
EOF
echo "Created .env and .rally-data. The first owner signs up using RALLY_BOOTSTRAP_SECRET from .env."
