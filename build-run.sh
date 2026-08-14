#!/usr/bin/sh
set -euo pipefail
podman build -t localhost/blog:latest --target runtime -f .devcontainer/Dockerfile .
podman run --rm -p 127.0.0.1:3000:3000 localhost/blog:latest
