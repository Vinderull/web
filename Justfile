# ── web Justfile ────────────────────────────────────────────────────────────
# Single source of truth for the Rust command flags used by the devcontainer
# and CI (`devcontainers/ci` runs `just <recipe>` inside the dev container).
# Host-side recipes cover the podman image, Flatcar/Caddy validation, the
# vendored htmx file, and the release workflow.
#
# `just` with no arguments prints this grouped recipe list (`just --list
# --unsorted` keeps the group order below). Nothing in this file pushes,
# signs, or attests images — those stay in .github/workflows/ci.yml.

image_tag := "localhost/blog:latest"

# default: print the grouped recipe list
default:
    @just --list --unsorted

# ── devcontainer: run inside the dev container (Rust toolchain) ─────────────

# build: compile with locked dependencies
[group('devcontainer')]
build:
    cargo build --locked

# run: run the server (port 3000 is forwarded)
[group('devcontainer')]
run:
    cargo run --locked

# test: run the test suite with locked dependencies
[group('devcontainer')]
test:
    cargo test --locked

# fmt: format all sources
[group('devcontainer')]
fmt:
    cargo fmt --all

# fmt-check: verify formatting
[group('devcontainer')]
fmt-check:
    cargo fmt --all -- --check

# clippy: lint all targets with warnings denied
[group('devcontainer')]
clippy:
    cargo clippy --all-targets --locked -- -D warnings

# rust-check: fmt-check, build, test, and clippy in one pass
[group('devcontainer')]
rust-check: fmt-check build test clippy

# check: full non-mutating verification (rust-check + vendored htmx)
[group('devcontainer')]
check: rust-check htmx-check

# ── host: run on the host (podman, butane, caddy, gh) ───────────────────────

# htmx-update: refetch and vendor the pinned htmx release (mutates static/js/htmx.min.js)
[group('host')]
htmx-update:
    scripts/update-htmx.sh

# htmx-check: verify static/js/htmx.min.js matches the pinned htmx release
[group('host')]
htmx-check:
    scripts/update-htmx.sh --check

# image-build: build the scratch runtime image with podman (tag: {{image_tag}})
[group('host')]
image-build:
    podman build -t {{ image_tag }} --target runtime -f .devcontainer/Dockerfile .

# image-run: image-build, then run loopback-only on http://127.0.0.1:3000
[group('host')]
image-run: image-build
    podman run --rm -p 127.0.0.1:3000:3000 {{ image_tag }}

# image-build-clean: warn, run global `buildah prune --force`, then rebuild with --no-cache
[group('host')]
image-build-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    # Purge the --mount=type=cache buckets (/var/tmp/buildah-cache-<uid>);
    # podman system prune --external does NOT touch them. Also drops
    # intermediate images and build containers, not your images.
    echo "warning: running global 'buildah prune --force' (build cache, intermediate images, build containers)" >&2
    buildah prune --force
    podman build --no-cache -t {{ image_tag }} --target runtime -f .devcontainer/Dockerfile .

# flatcar-check: validate flatcar.bu with Butane's strict mode (as CI does)
[group('host')]
flatcar-check:
    butane --strict --pretty -d . flatcar.bu -o /dev/null

# flatcar-render: render ignition.json from flatcar.bu (mutates ignition.json)
[group('host')]
flatcar-render:
    butane --pretty -d . flatcar.bu -o ignition.json

# caddy-check: format-diff then validate the Caddyfile (as CI does)
[group('host')]
caddy-check:
    caddy fmt --diff Caddyfile
    caddy validate --config Caddyfile --adapter caddyfile

# ── release: version-gated publish (mutation is explicit, never default) ───

# release-check VERSION: verify VERSION matches Cargo.toml, then run the canonical preflight
[group('release')]
release-check VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    version="{{ VERSION }}"
    if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "error: VERSION must be a plain semver like 1.2.3 (got '$version')" >&2
        exit 2
    fi
    cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n1)"
    if [[ -z "$cargo_version" ]]; then
        echo "error: could not read the package version from Cargo.toml" >&2
        exit 1
    fi
    if [[ "$version" != "$cargo_version" ]]; then
        echo "error: VERSION '$version' does not match Cargo.toml package version '$cargo_version'" >&2
        exit 1
    fi
    echo "ok: $version matches Cargo.toml"
    # Canonical preflight: the same recipes CI runs, in the same devcontainer
    # (devcontainer up/exec directly — no host-local Rust toolchain required).
    devcontainer up --workspace-folder .
    devcontainer exec --workspace-folder . just rust-check
    just htmx-check

# release VERSION: create GitHub release vVERSION at main (runs release-check first; the only mutation)
[group('release')]
release VERSION: (release-check VERSION)
    gh release create v{{ VERSION }} --generate-notes --target main
