# Chores container image.
#
# The binary resolves everything relative to its working directory (chores.db,
# photos/, thumbnails/, logs/, static/, migrations/), so the image ships the app
# under /app and the entrypoint links static/ + migrations/ into the data dir —
# the same thing nix/module.nix's ExecStartPre does. Mount your data dir at
# /var/lib/chores.

FROM rust:1-bookworm AS build
WORKDIR /src
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libsqlite3-dev \
 && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release --locked --bin chores --bin migrate

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata libsqlite3-0 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/chores /src/target/release/migrate /app/bin/
COPY static /app/static
COPY migrations /app/migrations
COPY docker/entrypoint.sh /app/entrypoint.sh

ENV PORT=4001 \
    TZ=UTC \
    DATABASE_URL=/var/lib/chores/chores.db
WORKDIR /var/lib/chores
VOLUME /var/lib/chores
EXPOSE 4001
ENTRYPOINT ["/app/entrypoint.sh"]
