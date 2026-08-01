# Architecture

A static-ish personal blog: posts are Markdown files with TOML frontmatter,
rendered to HTML once at startup and served from memory. The runtime is a
single statically-linked binary sandboxed with Linux landlock, fronted in
production by Caddy for TLS.

```
                 ┌─────────── Browser (htmx) ───────────┐
                 │   hx-boost: navigations swap <body>    │
                 └───────────────────┬───────────────────┘
                                     │ HTTPS
                          ┌──────────▼──────────┐
                          │   Caddy (rev proxy) │  TLS 80/443 → :3000
                          │   zstd/gzip, health  │
                          └──────────┬──────────┘
                                     │ HTTP (pod localhost)
            ┌────────────────────────▼────────────────────────┐
            │  axum 0.8 server  (distroless, UID 65532)        │
            │  Router · State<Arc<Vec<Post>>> · TraceLayer     │
            │  ┌──────────┬────────────┬──────────┬────────┐  │
            │  │ /        │ /posts/{slug}│ /healthz │ /static│  │
            │  └────┬─────┴──────┬──────┴──────────┴───┬────┘  │
            │       │            │                       │       │
            │  Askama render   Askama render          ServeDir  │
            │  (index.html)   (post.html)            (tower-http)│
            │       \            /                       │       │
            │   in-memory pre-rendered HTML            static/   │
            │   (Arc<Vec<Post>>, loaded at boot)        (RO FS)  │
            └────────────────────────────────────────────┼───────┘
                                                         │
                              landlock: only static/ readable; rest denied
```

## Layers

### 1. Content layer — Markdown + TOML frontmatter (`content/posts/`)
Posts are `*.md` files with `+++`-delimited TOML frontmatter (`title`,
`date`, optional `description`). No database; the filesystem is the source of
truth. Posts are read **once at startup**, never at request time.

### 2. Post pipeline (`src/posts.rs`)
`load_all(dir)` walks `dir/posts`, and for each `.md` file:
1. `split_frontmatter` — splits on `+++` markers (lenient: unclosed/empty
   blocks fall back to raw content).
2. `toml::from_str` → `FrontMatter { title, date, description }`.
3. `render_markdown` — `pulldown-cmark` with tables, footnotes, strikethrough,
   task lists enabled; output is HTML strings. In the same parse pass it
   collects headings, slugifies them into stable `id` anchors (deduped on
   collision), writes those ids back onto the `<h1>`–`<h6>` tags, and produces
   a nested **table of contents** (`h2+` only; `h1` is the post title). No
   syntax-highlighting pass.
4. `format_date` — `YYYY-MM-DD` → `"[month repr:long] [day], [year]"` via the
   `time` crate.

Result: `Vec<Post>` sorted by date **descending**, each `Post` carrying its
slug, title, display date, optional description, pre-rendered body HTML, and
**pre-rendered ToC HTML**. Rendering never happens on the request path.

### 3. Template layer — Askama compile-time templates (`templates/`)
Three templates compiled by Askama at build time (type-checked, zero runtime
parsing):
- `base.html` — shared layout: `<head>`, `<header>`, `<main>` block,
  `<footer>` (copyright + MIT notice), loads `/static/css/main.css` and
  `/static/js/htmx.min.js`, sets `hx-boost="true"` on `<body>`.
- `index.html` — extends `base`, lists all posts (title + date → `/posts/{slug}`)
  and a search field that `hx-get`s `/search`, swapping the list in place.
- `post.html` — extends `base`, renders one post's pre-baked HTML via
  `{{ post.html|safe }}`, a precomputed `{{ post.toc|safe }}` nav (empty posts
  omit it), and an optional `<meta description>` in the `head` block.

Rust sides (`src/templates.rs`): `IndexTemplate<'a> { posts: &'a [Post] }` and
`PostTemplate<'a> { post: &'a Post }` — they borrow the in-memory `Post`s.

### 4. Web server layer — axum 0.8 + tower-http (`src/main.rs`)
A `Router` with shared `AppState { posts: Arc<Vec<Post>> }`:
| Route | Handler | Returns |
|-------|---------|---------|
| `GET /` | `index` | rendered `index.html` (`Html<String>`) |
| `GET /posts/{slug}` | `post` | rendered `post.html`; O(n) search → `404` if miss |
| `GET /search` | `search` | htmx fragment `<ul id="post-list">`; scans in-memory posts, `no-store` |
| `GET /healthz` | inline | `"ok"` as `text/plain` (Caddy health gate) |
| `GET /static/*` | `ServeDir` | files streamed from `static/` (tower-http `fs`) |

- `TraceLayer::new_for_http()` (tower-http `trace`) wraps every request in a
  tracing span for structured per-request logging.
- Posts are immutable after load and shared cheaply via `Arc`; handlers take
  `State<AppState>` (clones the `Arc`).
- Render failures map to `500`; unknown slug to `404`.

### 5. Frontend layer — htmx 2.0 + CSS (`static/`)
- `htmx.min.js` is **self-hosted** (vendored, not a CDN) and loaded on every
  page. `hx-boost="true"` on `<body>` upgrades same-origin links/forms into
  background fetches that swap `<body>` — giving SPA-like navigation without a
  client framework or a JS build step.
- `main.css` is a single hand-written stylesheet (max-width layout, post list,
  code/blockquote styling, `.htmx-indicator` transition). No CSS framework.

### 6. Runtime hardening layer — Landlock (`src/sandbox.rs`)
Before the tokio runtime is created, `sandbox::apply(static_dir)` restricts
the process with a Linux landlock ruleset (ABI V1 for broad kernel support):
- **Filesystem**: read-only access to `static_dir` only. `content/` is already
  in memory, the binary/templates/etc. become unreadable.
- **Network**: `BindTcp` + `ConnectTcp` allowed (the listener is bound first;
  after sandboxing new binds/connects are the only network ops permitted).
- Linux-only; non-Linux logs a warning and runs unsandboxed. Supported-but-
  unenforced is a hard error (server refuses to start). Worker threads spawned
  by the tokio runtime inherit the restricted domain.

### 7. Process / startup layer (`src/main.rs`, `src/config.rs`)
Startup order is deliberate (each step justifies the next):
1. **Tracing init** — `tracing-subscriber` fmt, `EnvFilter` from `RUST_LOG`
   (default `info`).
2. **Config** — `Config::from_env()` reads `BIND_ADDR` (`0.0.0.0:3000`),
   `CONTENT_DIR` (`content`), `STATIC_DIR` (`static`).
3. **Load posts** — `posts::load_all` (disk I/O, pre-sandbox).
4. **Bind TCP listener** — `std::net::TcpListener::bind`, set nonblocking
   (pre-sandbox so landlock can't block it).
5. **Apply sandbox** — landlock restricts FS + net.
6. **Build tokio runtime** — multi-thread, created post-sandbox so its worker
   threads inherit the landlock domain.
7. **Serve** — `axum::serve` with `with_graceful_shutdown` awaiting SIGINT
   (Ctrl-C) or SIGTERM (Unix); in-flight requests drain before exit.

### 8. Build & deployment layer
- **Dockerfile** (`.devcontainer/Dockerfile`) — multi-stage: `dev` (devcontainer
  toolchain + musl target), `builder` (`cargo build --release` against
  `x86_64-unknown-linux-musl`), `runtime` (distroless
  `static-debian13:nonroot`, copies binary + `content/` + `static/`, bakes
  `CONTENT_DIR`/`STATIC_DIR` via `ENV`, runs as UID 65532, `EXPOSE 3000`).
  Final image ~2.3MB, no shell/libc/package-manager.
- **Podman Quadlet** (`quadlet/`) — systemd units (`web.pod`,
  `blog.container`, `caddy.container`, `*.volume`) run the app + Caddy in one
  pod. Blog is reachable on `127.0.0.1:3000` inside the shared namespace;
  Caddy publishes 80/443 and proxies. Read-only roots, all caps dropped,
  `Restart=always`. Caddy self-gates routing on `/healthz`.
- **CI** (`.github/workflows/ci.yml`) — fmt/test/clippy via devcontainers,
  a `docker-build` job that cosign-verifies the distroless base image, and a
  release-triggered `deploy` job (fires on GitHub Release publish against a
  `v*` tag) that builds+pushes the `runtime` image to
  `ghcr.io/<owner>/web:latest` and `:<tag>`, then keyless-signs it with
  cosign and attaches a SLSA provenance attestation. `policy.json` enforces
  the signature on pull.
- **Flatcar** (`flatcar.bu`) — Ignition provisioning: writes the quadlet units
  and `Caddyfile` to `/etc`, enables the `flatcar-podman` sysext. The blog image
  is pulled from the **public** GHCR package by the quadlet (`Update=registry`,
  unauthenticated); no on-box build/load step. Supply-chain trust comes from
  the cosign signature enforced by `policy.json`.

## Request lifecycle (`GET /posts/my-post`)
Tracing span opens → router matches `/posts/{slug}` → axum injects cloned
`State<Arc<Vec<Post>>>` and `Path(slug)` → handler searches posts → on hit,
`PostTemplate { post }` renders against `post.html` (Askama) → `Html<String>`
returned as `text/html`; miss → `404`; render error → `500`. Static assets hit
`ServeDir` reading from the only landlock-readable path. SIGTERM/SIGINT drains
in-flight requests, then the process exits.

## Key invariants
- **Posts are immutable and in-memory.** A new/changed post requires a restart
  (re-run `load_all` at boot).
- **The request path touches the filesystem only for `/static/*`.** Everything
  else is served from RAM, so landlock can deny the rest.
- **No outbound network after sandboxing** except what the listener already
  needs; the blog makes no external calls.
- **Templates are compile-time checked** — a bad template fails the build, not
  a request.
