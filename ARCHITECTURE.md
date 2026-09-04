# Architecture

A static-ish personal blog: posts are Markdown files with TOML frontmatter,
rendered to HTML once at startup and served from memory. The runtime is a
single statically-linked binary sandboxed with Linux landlock, fronted in
production by Caddy for TLS. Site name/URL are compile-time constants
(`SITE_NAME` = "My Bloginorium", `SITE_URL` = "https://bloginorium.me").

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
            │  axum 0.8 server  (scratch, UID 65532)           │
            │  Router · AppState                               │
            │  ┌───────────┬───────────┬──────────┬────────┐  │
            │  │ /  /about │/posts/{s}│/tags /tags/{t}      │  │
            │  │ /search   │/feed.xml │ /teapot  │/healthz │  │
            │  └─────┬─────┴─────┬─────┴──────────┴───┬────┘  │
            │        │           │                      │       │
            │   Pre-rendered    Pre-rendered      Embedded static│
            │   HTML + ETag     HTML + ETag       assets: /static/│
            │   (built at boot, (hashmap lookup)   css/js/favicon│
            │    hashmap lookup)                  (compile-time)│
            └───────────────────────────────────────┼──────────┘
                                                     │
                 landlock: no filesystem path granted; all denied
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
2. `toml::from_str` → `FrontMatter { title, date, description, tags }`.
3. `render_markdown` — `pulldown-cmark` with tables, footnotes, strikethrough,
   task lists enabled; output is HTML strings. In the same parse pass it
   collects headings, slugifies them into stable `id` anchors (deduped on
   collision), writes those ids back onto the `<h1>`–`<h6>` tags, and produces
   a nested **table of contents** (`h2+` only; `h1` is the post title). Fenced
   code blocks pass through `pulldown-cmark` unchanged: it emits `<pre><code>`
   (with a `language-*` class for the fence's language identifier) and escapes
   the code; no syntax coloring is applied. The final HTML is sanitized through
   `ammonia`
   (allowlists safe elements/attributes; strips everything else).
4. `format_date` — `YYYY-MM-DD` → `"[month repr:long] [day], [year]"` via the
   `time` crate.
5. `reading_time` — word count ÷ 200, rounded up to nearest integer.
6. `tags` is a flat `Vec<String>` (default empty).

Result: `Vec<Post>` sorted by date **descending**, each `Post` carrying its
slug, title, display date, optional description, pre-rendered body HTML,
**pre-rendered ToC HTML**, tags, and reading time. Rendering never happens on
the request path.

### 3. Template layer — Askama compile-time templates (`templates/`)
Eight templates compiled by Askama at build time (type-checked, zero runtime
parsing):
- `base.html` — shared layout: `<head>`, `<header>`, `<main>` block,
  `<footer>` (copyright + MIT notice), loads `/static/css/main.css` and
  `/static/js/htmx.min.js`, sets `hx-boost="true"` on `<body>`.
- `index.html` — extends `base`, lists all posts (title + date → `/posts/{slug}`)
  and a search `GET` form that works with or without JavaScript.
- `post.html` — extends `base`, renders one post's pre-baked HTML via
  `{{ post.html|safe }}`, a precomputed `{{ post.toc|safe }}` nav (empty posts
  omit it), an optional `<meta description>` in the `head` block, flat
  `tags` rendered as links to `/tags/{tag}`, reading time, and
  previous/next post navigation links.
- `tags.html` / `tag.html` — extends `base`; the former lists every tag (with
  post counts) linking to `/tags/{tag}`, the latter lists the posts for one tag.
  Both pre-rendered at boot.
- `page.html` — extends `base`, renders a standalone page (e.g. `/about`) from
  `content/pages/*.md`. Pages have no date/tags/ToC/reading time.
- `search_results.html` — bare fragment (no `<html>`): `<ul>` of matching
  posts, used by htmx search swaps. Includes an empty-state message when no
  posts match.
- `404.html` — styled 404 page extending `base`, shown on unknown routes and
  unknown slugs/tags.

Rust side (`src/templates.rs`): `IndexTemplate<'a>` (posts + query + site_name),
`PostTemplate<'a>` (post + newer/older + site_name), `PageTemplate<'a>` (page +
site_name), `SearchResultsTemplate<'a>` (posts + query), `TagsIndexTemplate<'a>`
(tags + site_name), `TagTemplate<'a>` (tag + posts + site_name),
`NotFoundTemplate<'a>` (site_name). All borrow the in-memory `Post`s/`Page`s.

### 4. Web server layer — axum 0.8 (`src/lib.rs`)
A `Router` with shared `AppState` containing pre-rendered `Bytes` + `ETag`
values for every page, built once in `build_app()`. Request handlers do pure
hashmap lookups + ETag comparisons — no per-request Askama rendering.

| Route | Handler | Returns |
|-------|---------|---------|
| `GET /` | `index` | pre-rendered index HTML, `304` on matching ETag |
| `GET /posts/{slug}` | `post` | pre-rendered post; hashmap lookup → `404` if miss |
| `GET /search` | `search` | htmx fragment or full page; `no-store`; linear scan |
| `GET /tags` | `tags_index` | pre-rendered tags index (all tags + counts) |
| `GET /tags/{tag}` | `tag` | pre-rendered tag page; hashmap lookup → `404` if unknown |
| `GET /about` | `about` | pre-rendered about page (404 if no `content/pages/about.md`) |
| `GET /teapot` | `teapot` | `418 I'm a teapot` easter egg (text/plain, no caching) |
| `GET /feed.xml` | `feed` | pre-rendered Atom feed (`application/atom+xml`) |
| `GET /feed` | redirect | permanent redirect to `/feed.xml` |
| `GET /healthz` | inline | `"ok"` as `text/plain` (Caddy health gate) |
| `GET /static/css/main.css` | `static_css` | embedded `include_bytes!` body, `text/css; charset=utf-8` |
| `GET /static/js/htmx.min.js` | `static_js` | embedded `include_bytes!` body, `text/javascript; charset=utf-8` |
| `GET /static/favicon.svg` | `static_favicon` | embedded `include_bytes!` body, `image/svg+xml` |
| (fallback) | `not_found` | pre-rendered 404 page (incl. other `/static/*`) |

- All mutable work (markdown → HTML, Askama rendering, feed XML generation)
  happens in `build_app()` before the router is built. The request path only
  does hashmap lookups and ETag comparisons.
- ETags are deterministic xxh3 hashes of the rendered HTML `Bytes`, so they
  are stable across restarts and deploys — cached clients keep getting `304`s.
- Cached routes set `Cache-Control: public, max-age=43200, s-maxage=43200, immutable`.
- Security headers on every HTML response: `X-Content-Type-Options: nosniff`,
  `X-Frame-Options: DENY`, `Referrer-Policy: strict-origin-when-cross-origin`,
  `Content-Security-Policy: default-src 'self'; style-src 'self'; script-src 'self'`.

### 5. Frontend layer — htmx 4.0.0 + CSS (`static/`)
The repo's `static/` directory is a **compile-time source**: `main.css`,
`htmx.min.js`, and `favicon.svg` are embedded into the binary with
`include_bytes!` and served from RAM at their exact `/static/...` URLs, so no
`static/` copy is needed at runtime.
- `htmx.min.js` is **self-hosted** (vendored, not a CDN) and loaded on every
  page. `hx-boost="true"` on `<body>` upgrades same-origin links/forms into
  background fetches that swap `<body>` — giving SPA-like navigation without a
  client framework or a JS build step.
- `main.css` is a single hand-written stylesheet (max-width layout, post list,
  code/blockquote styling, `.htmx-indicator` transition). No CSS framework.

### 6. Runtime hardening layer — Landlock (`src/sandbox.rs`)
Before the tokio runtime is created, `sandbox::apply()` restricts
the process with a Linux landlock ruleset (ABI V9, `BestEffort` compatibility):
- **Filesystem**: **no filesystem path is granted.** `content/` is already in
  memory and the static assets are embedded into the binary at compile time,
  so the sandbox denies every further read and write.
- **Network**: `BindTcp` and `ConnectTcp` are both handled with no port-grant
  rules, so landlock denies all TCP binding (the listener is already bound
  before sandboxing) and all outbound connects (the server makes no external
  calls).
- **Scope**: abstract UNIX sockets and cross-process signals are scoped to the
  sandbox domain — the blog connects to no UNIX sockets and signals no other
  process, so both are confined against lateral movement within the pod.
- Linux-only; non-Linux logs a warning and runs unsandboxed. Supported-but-
  unenforced is a hard error (server refuses to start). Worker threads spawned
  by the tokio runtime inherit the restricted domain.

### 7. Process / startup layer (`src/main.rs`, `src/config.rs`)
Startup order is deliberate (each step justifies the next):
1. **Config** — `Config::from_env()` reads `BIND_ADDR` (`0.0.0.0:3000`),
   `CONTENT_DIR` (`content`).
2. **Load posts** — `posts::load_all` (disk I/O, pre-sandbox).
3. **Bind TCP listener** — `std::net::TcpListener::bind`, set nonblocking
   (pre-sandbox so landlock can't block it).
4. **Apply sandbox** — landlock restricts FS + net.
5. **Build tokio runtime** — multi-thread, created post-sandbox so its worker
   threads inherit the landlock domain.
6. **Serve** — `axum::serve` with `with_graceful_shutdown` awaiting SIGINT
   (Ctrl-C) or SIGTERM (Unix); in-flight requests drain before exit.

### 8. Build & deployment layer
- **Dockerfile** (`.devcontainer/Dockerfile`) — multi-stage: `dev` (devcontainer
  toolchain + musl target), `builder` (`cargo build --release` against
  `x86_64-unknown-linux-musl`), `runtime` (scratch, copies binary + `content/`,
  bakes `CONTENT_DIR` via `ENV`, runs as UID 65532, `EXPOSE 3000`). The static
  assets are compiled into the binary, so no `static/` copy or env var is
  needed at runtime.
  Final image ~2.3MB, no shell/libc/package-manager.
- **Podman Quadlet** (`quadlet/`) — systemd units (`web.pod`,
  `blog.container`, `caddy.container`, `*.volume`) run the app + Caddy in one
  pod. Blog is reachable on `127.0.0.1:3000` inside the shared namespace;
  Caddy publishes 80/443 and proxies. Read-only roots, all caps dropped,
  `Restart=always`. Caddy self-gates routing on `/healthz`.
- **CI** (`.github/workflows/ci.yml`) — fmt/test/clippy via devcontainers,
  a `docker-build` job that smoke-tests the scratch `runtime` build, and a
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
Router matches `/posts/{slug}` → axum injects cloned `State<AppState>` and
`Path(slug)` → handler does hashmap lookup in `post_pages` → on hit, checks
`If-None-Match` header against stored ETag: match returns `304 Not Modified`,
miss returns `200` with the pre-rendered HTML bytes + `ETag` + cache headers;
slug not found → pre-rendered 404 page. Static asset requests hit their exact
routes and are served the byte-identical `include_bytes!` bodies straight
from RAM — no filesystem access. SIGTERM/SIGINT drains in-flight requests,
then the process exits.

## Key invariants
- **Posts are immutable and in-memory.** A new/changed post requires a restart
  (re-run `load_all` at boot).
- **The request path never touches the filesystem.** Everything — pages and
  the embedded static assets — is served from RAM, so landlock grants no
  filesystem path at all.
- **No outbound network after sandboxing**: `ConnectTcp` is handled, so egress
  connects are kernel-denied; the blog makes no external calls.
- **Templates are compile-time checked** — a bad template fails the build, not
  a request.