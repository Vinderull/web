# web

Personal blog built with Rust, axum, and htmx.

## Stack

- **axum** 0.8 — HTTP server
- **askama** 0.16 — compile-time HTML templates
- **htmx** 2.0 — progressive enhancement (SPA-like navigation via `hx-boost`)
- **pulldown-cmark** — Markdown rendering
- **syntect** — syntax highlighting at load time
- **ammonia** — HTML sanitization
- **atom_syndication** — Atom feed generation
- **landlock** — Linux kernel sandboxing (filesystem + network restrictions)
- **tower-http** — static file serving, request tracing

## Features

- **Pre-rendered at boot** — all pages, tag indexes, and the Atom feed are
  rendered once at startup. Requests do hashmap lookups, not per-request
  template rendering.
- **ETag-based caching** — every pre-rendered page has a deterministic xxh3
  ETag. Cached clients get `304 Not Modified` across deploys.
- **Atom feed** — `/feed.xml` with full-content entries, tag categories, and
  proper RFC 3339 timestamps.
- **Tags** — `/tags` lists all tags with post counts; `/tags/{tag}` shows
  matching posts.
- **Search** — htmx-powered live search (`/search`); falls back to a
  full-page request when JavaScript is disabled.
- **Previous/next navigation** — every post page links to the chronologically
  adjacent posts.
- **Reading time** — estimated from word count, shown on each post.
- **Syntax highlighting** — load-time `syntect` with CSS classes (no inline
  styles), CSP-friendly.
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
(target: `dev`) and the deployment image (final stage: `runtime`).

```bash
# Build and start the dev container (mounts your workspace, installs toolchain)
devcontainer up --workspace-folder .

# Run commands inside the dev container
devcontainer exec --workspace-folder . cargo build
devcontainer exec --workspace-folder . cargo test
devcontainer exec --workspace-folder . cargo run
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

Posts are loaded at startup. Restart the server to pick up new/changed posts.

## Landlock sandbox

The server sandboxes itself with Linux landlock before starting the tokio
runtime. Worker threads inherit the restrictions:

- **Filesystem**: read-only access to the static asset dir only. Posts are
  loaded into memory *before* the sandbox applies, so the runtime never reads
  `content/`. All writes denied.
- **Network**: TCP listener is bound *before* sandboxing. After sandboxing, all
  new `bind()`/`connect()` calls are denied (no outbound connections).

If the kernel doesn't support landlock, the server logs a warning and continues
without the sandbox. If landlock is supported but can't enforce, the server
refuses to start.

## Deployment

The runtime image is **distroless** (`gcr.io/distroless/static-debian13:nonroot`)
— no shell, no package manager, no libc. The binary is statically linked with
musl, so the final image is ~2.3MB and contains only the binary, content, and
static assets, running as a non-root user (UID 65532). `CONTENT_DIR` and
`STATIC_DIR` are baked in via the Dockerfile `ENV`.

Deployment targets Podman — the runtime image is built with `podman build` from
the same multi-stage Dockerfile (`runtime` stage) used in development.

### Option A: Podman Quadlet + Caddy (recommended for production)

`quadlet/` contains systemd-managed Podman units that run the blog app and a
[Caddy](https://caddyserver.com/) reverse proxy together in a single **pod**.
Caddy automatically provisions and renews Let's Encrypt TLS certificates, so
this is the path to HTTPS on a real domain. Quadlet does not build images, so
build the runtime image once and rebuild on code changes:

```bash
# Build the distroless runtime image
podman build -t localhost/blog:latest --target runtime -f .devcontainer/Dockerfile .
```

Then install the units, start the pod, and edit the `Caddyfile` domain. See
[`quadlet/README.md`](quadlet/README.md) for the full setup — rootful/rootless
variants and the Flatcar/VPS Ignition provisioning flow. Caddy terminates TLS
on ports 80/443 and proxies to the blog over the pod's shared localhost
namespace; the blog is not published to the host, so all external traffic goes
through Caddy. Caddy also adds `zstd`/`gzip` compression (see `Caddyfile`).

### Option B: plain podman (no reverse proxy)

```bash
# The Dockerfile lives in .devcontainer/ — use -f to point to it
podman build -t localhost/blog:latest --target runtime -f .devcontainer/Dockerfile .

# Publish to the host on all interfaces (0.0.0.0); LAN-visible in rootful mode.
podman run --rm -p 3000:3000 localhost/blog:latest

# Loopback-only: not exposed to the network. Prefer this for local dev.
podman run --rm -p 127.0.0.1:3000:3000 localhost/blog:latest

# Server starts on http://127.0.0.1:3000 (plain HTTP, no TLS)
```

For local testing the [`build-run.sh`](build-run.sh) helper does both of those
steps in one shot — it builds the `runtime` image (as `blog:latest`) and runs it
loopback-only on `127.0.0.1:3000`:

```bash
./build-run.sh
```

## Configuration

| Env var      | Default       | Description               |
|--------------|---------------|---------------------------|
| `BIND_ADDR`  | `0.0.0.0:3000`| Listen address            |
| `CONTENT_DIR`| `content`     | Path to posts directory   |
| `STATIC_DIR` | `static`      | Path to static assets     |
| `RUST_LOG`   | `info`        | Tracing filter            |
