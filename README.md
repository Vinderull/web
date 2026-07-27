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

Build the runtime image (uses the `runtime` stage of the multi-stage
Dockerfile) and run it:

```bash
# The Dockerfile lives in .devcontainer/ — use -f to point to it
docker build -t blog --target runtime -f .devcontainer/Dockerfile .
docker run -p 3000:3000 blog
# Server starts on http://localhost:3000
```

The runtime image is `debian:trixie-slim` with just the release binary,
content, and static assets, running as a non-root `blog` user. `CONTENT_DIR`
and `STATIC_DIR` are baked in via the Dockerfile `ENV`.

## Configuration

| Env var      | Default       | Description               |
|--------------|---------------|---------------------------|
| `BIND_ADDR`  | `0.0.0.0:3000`| Listen address            |
| `CONTENT_DIR`| `content`     | Path to posts directory   |
| `STATIC_DIR` | `static`      | Path to static assets     |
| `RUST_LOG`   | `info`        | Tracing filter            |
