#!/usr/bin/sh
set -e

NO_CACHE=
if [ "$1" = "--clean" ]; then
    # Purge the --mount=type=cache buckets (/var/tmp/buildah-cache-<uid>);
    # podman system prune --external does NOT touch them. Also drops
    # intermediate images and build containers, not your images.
    buildah prune --force
    NO_CACHE=--no-cache
fi

podman build $NO_CACHE -t localhost/blog:latest --target runtime -f .devcontainer/Dockerfile .
podman run --rm -p 127.0.0.1:3000:3000 localhost/blog:latest
