FROM docker.io/oven/bun:1 AS web
WORKDIR /web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ .
RUN bun run build

FROM docker.io/library/rust:1-trixie AS rust
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY runtime/skills runtime/skills
RUN cargo build --release --locked --bins

FROM docker.io/library/debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends podman git openssh-client ca-certificates curl libssl3t64 && rm -rf /var/lib/apt/lists/*
COPY --from=rust /source/target/release/rally /usr/local/bin/rally
COPY --from=rust /source/target/release/rally-server /usr/local/bin/rally-server
COPY --from=web /web/dist /opt/rally/web
COPY scripts/fetch-embeddings.sh /opt/rally/
RUN sh /opt/rally/fetch-embeddings.sh /opt/rally/embeddings
ENV RALLY_WEB_DIR=/opt/rally/web RALLY_EMBEDDING_DIR=/opt/rally/embeddings
ENTRYPOINT ["rally-server"]
