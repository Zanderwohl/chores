#!/bin/sh
# Mirror nix/module.nix's chores-setup: point static/ and migrations/ at the
# copies baked into this image (replacing any stale links), then run in place.
set -eu
ln -sfn /app/static static
ln -sfn /app/migrations migrations
mkdir -p photos thumbnails logs
exec /app/bin/chores --port "$PORT" --tz "$TZ" "$@"
