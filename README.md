# web

Personal blog built with Rust, axum, and htmx.

## Stack

- **axum** 0.8 — HTTP server
- **askama** 0.16 — compile-time HTML templates
- **htmx** 4.0.0 — progressive enhancement (SPA-like navigation via `hx-boost`)
- **pulldown-cmark** — simple-Markdown rendering: raw HTML and unsafe
  link/image destinations are rejected at startup; validated events render
  directly to HTML (no post-render sanitizer)
- **landlock** — Linux kernel sandboxing (filesystem + network restrictions)
- **embedded static assets** — CSS, vendored htmx JS, and the SVG favicon are
  baked into the binary at compile time

## Features

- **Pre-rendered at boot** — all pages, tag indexes, and the Atom feed are
  rendered once at startup. Requests do hashmap lookups, not per-request
  template rendering.
- **ETag-based caching** — every pre-rendered page has a deterministic xxh3
  ETag. Cached clients get `304 Not Modified` across deploys.
- **Atom feed** — `/feed.xml` with full-content entries, tag categories, and
  proper RFC 3339 timestamps. Generated with a hand-rolled writer plus
  RFC 4287 compliance tests.
- **Tags** — `/tags` lists all tags with post counts; `/tags/{tag}` shows
  matching posts.
- **Search** — htmx-powered live search (`/search`); falls back to a
  full-page request when JavaScript is disabled.
- **Previous/next navigation** — every post page links to the chronologically
  adjacent posts.
- **Reading time** — estimated from word count, shown on each post.
- **Code blocks** — fenced code is rendered and escaped by `pulldown-cmark`
  into `<pre><code>` with no syntax coloring.
- **Simple Markdown only** — authored raw HTML is not supported. Block and
  inline HTML are rejected when the post is loaded; links/images must use
  relative URLs, `#fragments`, or `http`/`https`/`mailto`. A violating post
  fails startup rather than being silently sanitized.
- **Custom 404** — styled 404 page for unknown routes, slugs, and tags.
- **Security headers** — `X-Content-Type-Options`, `X-Frame-Options`,
  `Referrer-Policy`, `Permissions-Policy`, and a strict CSP on every
  HTML response.
- **Standalone pages** — `/about` served from `content/pages/about.md`.
- **Easter egg** — `418 I'm a teapot` at `/teapot`.

## Project structure

```
content/posts/     Markdown posts with TOML frontmatter
content/pages/     Standalone pages (e.g. about.md -> /about)
static/            CSS, JS (htmx self-hosted)
templates/         Askama HTML templates
src/
  main.rs          Entry point: config, bind, sandbox, serve
  lib.rs           Router, handlers, AppState, pre-rendering, ETags
  config.rs        Environment-based configuration
  sandbox.rs       Landlock sandbox setup
  posts.rs         Post loading, frontmatter parsing, markdown rendering
  templates.rs     Askama template structs
  feed.rs          Atom feed generation
.devcontainer/     Multi-stage Dockerfile (dev + runtime), devcontainer config
quadlet/           Podman Quadlet units (systemd-managed pod: blog + Caddy)
```

## Development

Uses devcontainers. The same Dockerfile produces both the dev environment
(target: `dev`) and the deployment image (final stage: `runtime`). The
`Justfile` at the repo root is the single source of truth for the Rust command
flags, shared by the devcontainer and CI.

`just` (no arguments) prints the grouped recipe list. The groups:

| Group           | Runs where                          | Recipes |
|-----------------|-------------------------------------|---------|
| `devcontainer`  | inside the dev container (cargo)    | `build`, `run`, `test`, `fmt`, `fmt-check`, `clippy`, `rust-check`, `check` |
| `host`          | on the host (podman, butane, caddy) | `htmx-update`, `htmx-check`, `image-build`, `image-run`, `image-build-clean`, `flatcar-check`, `flatcar-render`, `caddy-check` |
| `release`       | on the host (gh, devcontainer CLI)  | `release-check`, `release` |

The `devcontainer` recipes run the Rust toolchain, which lives only inside
the container; `host` recipes need their tools directly on the host (podman,
butane, caddy; curl/openssl for htmx; `gh` and the `devcontainer` CLI for
release). `just check` aggregates the full non-mutating verification:
`rust-check` (fmt-check, build, test, clippy) plus `htmx-check`.

```bash
# Build and start the dev container (mounts your workspace, installs toolchain)
devcontainer up --workspace-folder .

# Canonical Rust recipes, run inside the dev container
devcontainer exec --workspace-folder . just fmt-check
devcontainer exec --workspace-folder . just clippy
devcontainer exec --workspace-folder . just test
devcontainer exec --workspace-folder . just run
# Server starts on http://localhost:3000
```

Port 3000 is forwarded automatically (configured in `devcontainer.json`).

### Writing posts

Create a Markdown file in `content/posts/` with TOML frontmatter:

```
+++
title = "My Post"
date = "2024-01-15"
description = "Optional description for SEO"
tags = ["rust", "web"]
+++

Markdown content here.
```

Posts are written in **simple Markdown**. Raw HTML — block or inline, inside
headings included — is not supported and causes a startup error if present.
Links and images may use relative URLs, `#fragments`, and case-insensitive
`http`, `https`, or `mailto` schemes. Protocol-relative targets (`//host/…`),
backslashes, ASCII whitespace/control characters in the target, and all other
explicit schemes (`javascript:`, `data:`, `file:`, `ftp:`, …) are rejected.

Posts are loaded at startup. Restart the server to pick up new/changed posts.
A post that violates the Markdown policy stops startup with an error naming
the file, so a bad post is never served half-sanitized.

## Landlock sandbox

The server sandboxes itself with Linux landlock before starting the tokio
runtime. Worker threads inherit the restrictions:

- **Filesystem**: posts are loaded into memory *before* the sandbox applies
  and the static assets are embedded into the binary at compile time, so the
  sandbox grants **no filesystem path**: all further reads and writes are
  denied.
- **Network**: TCP listener is bound *before* sandboxing. After sandboxing, all
  new `bind()`/`connect()` calls are denied (no outbound connections).

If the kernel doesn't support landlock, the server logs a warning and continues
without the sandbox. If landlock is supported but can't enforce, the server
refuses to start.

## Deployment

The runtime image is built `FROM scratch` — a fully static musl binary, no
shell, no libc, no `/etc/passwd`, no userspace of any kind. The image contains
only the binary and content, running as a non-root user (UID 65532).
`CONTENT_DIR` is baked in via the Dockerfile `ENV`; the static assets are
already embedded in the binary at compile time, so no static directory is
copied into the runtime image.

CI (`.github/workflows/ci.yml`) runs the required validation jobs (fmt/test/
clippy via the Just recipes inside devcontainers, plus htmx-vendor and a native
Docker build) on pull requests before they can merge to `main`. On a published
GitHub Release, the `deploy` job verifies the release tag equals `v` + the
`Cargo.toml` package version, then builds the runtime image and pushes it to
`ghcr.io/vinderull/web:latest` (plus `:<tag>`). The Quadlet units pull from
GHCR (`Update=registry`), so no local build is needed for deployment.

### Option A: Podman Quadlet + Caddy (recommended for production)

`quadlet/` contains systemd-managed Podman units that run the blog app and a
[Caddy](https://caddyserver.com/) reverse proxy together in a single **pod**.
Caddy automatically provisions and renews Let's Encrypt TLS certificates, so
this is the path to HTTPS on a real domain. Caddy terminates TLS on ports
80/443 and proxies to the blog over the pod's shared localhost namespace; the
blog is not published to the host, so all external traffic goes through Caddy.

See [`quadlet/README.md`](quadlet/README.md) for the full setup — rootless
systemd Quadlet units and the Flatcar/VPS Ignition provisioning flow.

To build locally (e.g. for testing before a release), tag the image to match
what the quadlet pulls:

```bash
just image_tag=ghcr.io/vinderull/web:latest image-build
```

### Option B: plain podman (no reverse proxy)

The `host` Just recipes build and run the `runtime` image with podman (the
Dockerfile lives in `.devcontainer/`):

```bash
# Build only; defaults to tag localhost/blog:latest
just image-build

# Build, then run loopback-only (not exposed to the network): http://127.0.0.1:3000
just image-run          # preferred for local dev

# Same build after purging buildah's caches and intermediate images (--no-cache)
just image-build-clean

# Publish to the host on all interfaces (0.0.0.0) instead
podman run --rm -p 3000:3000 localhost/blog:latest
# Server starts on http://127.0.0.1:3000 (plain HTTP, no TLS)
```

### Releasing

Releases are published from GitHub Actions, never locally — nothing in the
Justfile builds, pushes, signs, or attests images.

1. Bump `version` in `Cargo.toml` and commit.
2. `just release-check 1.2.3` — verifies the argument is plain semver and
   exactly equals the `Cargo.toml` package version, then runs the canonical
   preflight: `rust-check` inside the devcontainer (via `devcontainer
   up`/`exec`, no host-local toolchain needed) plus `htmx-check` on the host.
3. `just release 1.2.3` — the only mutating step:
   `gh release create v1.2.3 --generate-notes --target main`. CI checks the
   release tag against `Cargo.toml`, then builds, pushes, signs, and attests the
   runtime image in Actions.

## Configuration

| Env var      | Default       | Description               |
|--------------|---------------|---------------------------|
| `BIND_ADDR`  | `0.0.0.0:3000`| Listen address            |
| `CONTENT_DIR`| `content`     | Path to posts directory   |