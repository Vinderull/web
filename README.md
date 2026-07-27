# web

Personal blog built with Rust, axum, and htmx.

## Stack

- **axum** 0.8 — HTTP server
- **askama** 0.16 — compile-time HTML templates
- **htmx** 2.0 — progressive enhancement (SPA-like navigation via `hx-boost`)
- **pulldown-cmark** — Markdown rendering
- **landlock** — Linux kernel sandboxing (filesystem + network restrictions)
- **tower-http** — static file serving, request tracing

## Project structure

```
content/posts/     Markdown posts with TOML frontmatter
static/            CSS, JS (htmx self-hosted)
templates/         Askama HTML templates
src/
  main.rs          Entry point: config, bind, sandbox, serve
  config.rs        Environment-based configuration
  sandbox.rs       Landlock sandbox setup
  posts.rs         Post loading, frontmatter parsing, markdown rendering
  templates.rs     Askama template structs
.devcontainer/     Multi-stage Dockerfile (dev + runtime), devcontainer config
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

The runtime image is **distroless** (`gcr.io/distroless/static-debian12:nonroot`)
— no shell, no package manager, no libc. The binary is statically linked with
musl, so the final image is ~13MB and contains only the binary, content, and
static assets, running as a non-root user (UID 65532). `CONTENT_DIR` and
`STATIC_DIR` are baked in via the Dockerfile `ENV`.

### Option A: docker compose + Caddy (recommended for production)

`docker-compose.yml` brings up two services: the blog app and a
[Caddy](https://caddyserver.com/) reverse proxy in front of it. Caddy
automatically provisions and renews Let's Encrypt TLS certificates, so this is
the path to HTTPS on a real domain.

1. Edit `Caddyfile` and replace `blog.example.com` with your domain. The
   domain's DNS A/AAAA records must point at the host running the compose
   stack (Caddy completes an HTTP-01 challenge, so port 80 must be reachable
   from the public internet).
2. From the repo root:

```bash
# Builds the distroless runtime image (runtime stage) and starts Caddy
docker compose up -d --build

# Tail logs (watch Caddy obtain the cert on first boot)
docker compose logs -f
```

3. Caddy terminates TLS on ports 80/443 and proxies to `blog:3000` over the
   compose network. The blog service only exposes port 3000 *internally* — it
   is not published to the host, so all external traffic goes through Caddy.
   Caddy also adds `zstd`/`gzip` compression (see `Caddyfile`).

To run locally without TLS (e.g. for testing on `localhost`), replace the site
address in `Caddyfile` with `localhost` or `:80`; Caddy then serves plain HTTP
instead of provisioning certs.

### Option B: plain docker (no reverse proxy)

```bash
# The Dockerfile lives in .devcontainer/ — use -f to point to it
docker build -t blog --target runtime -f .devcontainer/Dockerfile .
docker run -p 3000:3000 blog
# Server starts on http://localhost:3000 (plain HTTP, no TLS)
```

## Configuration

| Env var      | Default       | Description               |
|--------------|---------------|---------------------------|
| `BIND_ADDR`  | `0.0.0.0:3000`| Listen address            |
| `CONTENT_DIR`| `content`     | Path to posts directory   |
| `STATIC_DIR` | `static`      | Path to static assets     |
| `RUST_LOG`   | `info`        | Tracing filter            |
