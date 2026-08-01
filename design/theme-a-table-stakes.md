# Theme A: Table Stakes

Making the blog feel complete without changing the architecture. Every feature
below is read-only — pre-rendered at boot and served from memory, or generated
on the request path by scanning `Arc<Vec<Post>>`. No persistent state, no
external services, no new failure modes.

---

## 1. Atom Feed

Generate an Atom XML document at boot from `Vec<Post>`, serve at `/feed.xml`.

**Decisions to make:**

- **Full-content vs summary-only.** Full-content lets people read in their
  reader; summary-only drives traffic back to the site. Most personal blogs
  ship full-content.
- **RSS autodiscovery.** A `<link rel="alternate"
  type="application/atom+xml" href="/feed.xml">` in the `<head>` lets
  browsers and feed readers auto-detect the feed.

**Also consider:** JSON Feed at `/feed.json`. Some readers (NetNewsWire,
Feedbin) prefer it, and indie devs like the format. Trivial to add alongside.

**Architecture:** Pure read. The feed template is rendered once at boot and
stored as pre-rendered HTML/XML in the same `Arc<HashMap>` that holds pages.
Zero per-request cost.

---

## 2. Sitemap

A single `sitemap.xml` listing all post URLs with `<lastmod>` dates.
Generated at boot. Helps search engines discover content.

Optionally add a `robots.txt` pointing to the sitemap and maybe disallowing
paths you don't want indexed.

**Question:** Do you *want* search engines? Some personal blogs deliberately
stay under the radar. If yes, sitemap + robots.txt is the standard way to
invite them.

**Architecture:** Same as the Atom feed — rendered once, stored in memory.

---

## 3. OpenGraph & Twitter Cards

When someone shares a post on Discord, Slack, Twitter, or iMessage, the
platform unfurls a preview. That preview comes from `<meta>` tags in the
`<head>`.

**Tags needed:**
- `og:title`, `og:description`, `og:image`, `og:url`, `og:type`
- `twitter:card`, `twitter:title`, `twitter:description`, `twitter:image`

The tags go in the `{% block head %}` of `post.html`. The base template
gets fallback values (site name, generic description).

**The hard part: `og:image`.** Every post needs a unique image for rich
previews, but this is a markdown-only blog with no image pipeline. Options:

| Approach | Effort | Result |
|----------|--------|--------|
| **Single static image** — avatar or site logo for every post | Trivial | Functional, not eye-catching |
| **Dynamic OG image generator** — render a PNG with post title on a styled background at boot using the `image` crate + a font | Medium | Polished, unique per post |
| **First image in the post** — if `![](img.png)` exists, use it | Low | Fragile, not all posts have images |
| **No image for now** — OG tags work without one, you get a plain text preview | Zero | Fine for a personal blog |

If dynamic OG images are desired, the `image` crate + `rusttype` can render
text on a background at boot. Store the result in `static/og/` (which already
has read-only Landlock access). The template references `/static/og/{slug}.png`.

**Architecture:** Template change only. If dynamic images are added, they're
generated during `load_all()` and written to the static dir before sandboxing.

---

## 4. Syntax Highlighting

Code blocks currently have a gray background with no token-level coloring.
Syntax highlighting assigns colors to keywords, strings, types, comments,
etc. based on language.

**Approach: build-time with `syntect`.** The `syntect` crate runs during
`load_all()`. It's fast, pure Rust, no JS shipped to clients. Output is
inline-styled HTML (`<span style="color: #...">`), which works with the
existing CSP. Done once at boot, served from memory.

**Alternative: client-side with Prism/highlight.js.** Simpler to implement
but adds JS weight, causes flash-of-unstyled-code, and requires loosening
the CSP to allow inline styles or `unsafe-inline`.

**Design decision: which color theme?** `syntect` bundles Sublime Text
themes. Options:
- A dark theme on a light blog creates striking contrast (e.g. `base16-ocean.dark`)
- A muted, warm theme that matches the site's restrained aesthetic
- Ship both and let `prefers-color-scheme` pick (requires rendering both and
  toggling visibility with CSS)

**Trade-off:** `syntect` adds compilation time (it bundles syntax definition
files) and ~2-3MB to the binary. For a personal blog this is fine; for the
~2.3MB distroless purist it's worth measuring before committing.

**Architecture:** Runs in `load_all()` during the markdown→HTML pass. The
rendered HTML includes highlighted code blocks. No request-path changes.

---

## 5. Dark Mode

The site currently has a single color scheme: near-white background
(`#fafafa`), near-black text (`#1a1a1a`). Dark mode inverts this.

**Step 1: CSS custom properties.** Move colors into variables:

```css
:root {
  --bg: #fafafa;
  --text: #1a1a1a;
  --text-muted: #666;
  --border: #ddd;
  --code-bg: #f0f0f0;
  --link: #0066cc;
}
```

**Step 2: Dark values.** Under `[data-theme="dark"]` or
`@media (prefers-color-scheme: dark)`:

```css
[data-theme="dark"] {
  --bg: #1a1a1a;
  --text: #e0e0e0;
  --text-muted: #999;
  --border: #333;
  --code-bg: #2a2a2a;
  --link: #66b3ff;
}
```

**Step 3: Toggle or no toggle?** Two philosophies:

- **Zero-JS purist approach** — only respect `prefers-color-scheme`. The OS
  controls the theme. No toggle, no localStorage, no JS. Fits the current
  architecture perfectly.
- **Toggle with localStorage** — a small sun/moon button in the header that
  flips `data-theme` on `<html>` and persists the choice. Requires ~10 lines
  of vanilla JS. Eliminates flash-of-wrong-theme on load. The current site
  has no custom JS at all (only the vendored htmx), so this is a meaningful
  philosophical choice.

**Syntax highlighting interaction:** If using `syntect`, the highlight theme
needs a dark variant. Options: render both light and dark HTML and toggle
with CSS (`display: none`), or choose a single theme that looks acceptable on
both backgrounds.

**Architecture:** CSS change only if going zero-JS. Adding a toggle introduces
a small JS file in `/static/js/theme.js`.

---

## 6. Tags / Categories

Each post gets an optional `tags = ["rust", "linux"]` in its TOML frontmatter.

**What this unlocks:**

- **Tag index pages** — `/tags/rust/` lists all posts with that tag. Generated
  at boot like the index page.
- **Tag listing** — a tag cloud or list on the homepage sidebar (or footer)
  showing available topics.
- **Per-tag Atom feeds** — `/tags/rust/feed.xml`. Power users love subscribing
  to specific topics.
- **Related posts** — "More about Rust" at the bottom of each post. Simple
  heuristic: posts sharing the most tags, limited to 3.

**Design decision: flat or hierarchical?** Flat tags (`rust`, `linux`) are
simpler and more common in personal blogs. Hierarchical (`programming/rust`)
adds complexity without much benefit at this scale. Go flat.

**Frontmatter syntax:**

```toml
+++
title = "Some Post"
date = "2024-01-15"
tags = ["rust", "axum", "web"]
+++
```

**Architecture:** At boot, build a `HashMap<String, Vec<usize>>` mapping tag
→ indices into the post list. Render a page per tag, plus the tag cloud on
the index. If per-tag feeds are desired, render those too. All pre-rendered.
A few hundred lines of Rust, mostly template code.

**Number of tags to support:** A personal blog with ~50 posts might have
10-20 unique tags. The pre-render cost is negligible. The tag cloud display
should handle the case gracefully — alphabetical sort, maybe size-by-count.

---

## 7. Previous/Next Navigation

At the bottom of each post: "← Previous Post Title" and "Next Post Title →".

Posts are already sorted by date descending in a `Vec`. The previous/next
are just `posts[i-1]` and `posts[i+1]`. No wrapping — the newest post has
no "next," the oldest has no "previous." That's correct.

The navigation goes at the bottom of the post template, styled as a flex row
with the previous left-aligned and next right-aligned.

**Architecture:** Trivial template change. The `PostTemplate` struct already
has access to the full post list (or can be given neighbor references). The
navigation is pre-rendered as part of the post page.

---

## 8. Reading Time

"4 min read" displayed next to the date on both the index and the post header.

Formula: word count ÷ 200 words per minute, rounded up. (200 is the lower
end of the typical 200-250 range — errs on the side of generosity to the
reader.)

Add a `read_minutes: u32` field to the `Post` struct, computed during
`load_all()` from the plaintext word count (strip HTML tags, count words).

**Architecture:** One field, one computation during load, two template
insertions. Zero overhead. Readers genuinely appreciate this for triaging
what to read.

---

## 9. Custom 404 Page

A styled page with "Not found" and a list of recent posts to explore.

Currently a 404 just returns the status code. Instead, render a `NotFound`
template at boot (like the index and post templates), and serve it when
`/posts/{slug}` misses.

The template includes:
- A message (mildly amusing is good — a haiku, an ASCII art, "you found
  the void")
- A list of the 5 most recent posts as quick recovery paths
- The same header/footer as other pages

**Architecture:** New template + handler branch. Pre-rendered at boot.

---

## 10. Search — htmx-Powered Server-Side

The most architecturally interesting feature in this set.

**Approach: server-side substring scan, swapped in with htmx.**

The index page (or a search bar in the header) gets an `<input>` that
`hx-get`s to `/search?q=...` on each keystroke with a debounce. The
server scans `Arc<Vec<Post>>` for title and description matches, renders
an HTML fragment of results, and htmx swaps it into the page.

**Why this fits:**
- Uses what already exists: htmx, in-memory posts, server-side rendering
- No client-side index, no WASM, no additional JS
- The search is just another read-only handler — zero new state
- Substring scan over ~100 posts is instant

**Alternative approaches considered and rejected for now:**

| Approach | Why not |
|----------|---------|
| Tinysearch (WASM) | Adds build complexity, WASM CSP adjustment, opaque build step |
| Pagefind | External tool dependency, opaque index format |
| Client-side JSON dump | Ships full post plaintext to every visitor — wasteful for a search most won't use |

**HTMX pattern:**

```html
<!-- In base.html header or on index page -->
<input type="search"
       name="q"
       placeholder="Search posts..."
       hx-get="/search"
       hx-trigger="keyup changed delay:300ms"
       hx-target="#post-list"
       hx-swap="innerHTML">
```

The `/search` handler returns an HTML fragment (a `<ul>` of matching posts
or "No results"), which htmx swaps into `#post-list`.

**Edge cases:**
- Empty query → return full post list (or nothing, depending on preference)
- No matches → "No posts found" message
- Very short queries (1-2 chars) → probably match everything; debounce
  minimizes this
- Case-insensitive matching

**Architecture:** New handler in the router, no new state, no pre-rendering
(a search is inherently dynamic). This is the only feature in Theme A that
does real work on the request path, but it's scanning memory — no disk I/O,
no allocations beyond the response.

---

## 11. Quick Wins

These are small enough to list without deep exploration:

### Table of Contents
Auto-generated from `h2`/`h3` headings in the rendered HTML. Two approaches:
- **Build-time:** Parse the rendered HTML, extract heading IDs and text,
  generate a `<nav>` list. Inserted into the post template. Fits the
  pre-render model.
- **Client-side:** JS walks the DOM and builds a nav. Simpler code but adds JS.

### Print Stylesheet
A `@media print` block in `main.css` that hides header/footer/nav and
adjusts font size and margins. ~20 lines of CSS. Nice touch for people
who print-to-PDF.

### `rel="me"` Links
If you have Mastodon, GitHub, or Bluesky accounts, add them to the footer
or an about page with `rel="me"`. Mastodon uses this to verify your domain
(your profile gets a green checkmark). Zero effort, meaningful for IndieWeb
identity.

### Responsive Images
For any images in posts: `max-width: 100%`, `height: auto`, `loading="lazy"`.
Already mostly handled by good CSS, but `loading="lazy"` on post images
improves performance on image-heavy pages.

### Copy Code Button
A small button on code blocks that copies the content to clipboard.
Requires JS. The htmx-idiomatic approach: include a small vanilla JS
snippet. Or skip it — code blocks with horizontal scroll and monospace
styling are already functional.

---

## Implementation Order

Suggested sequencing if building these out:

| Priority | Feature | Why first |
|----------|---------|-----------|
| 1 | Reading time | One field, two templates, instant payoff |
| 2 | Previous/Next navigation | Trivial, improves reading flow |
| 3 | Atom feed (+ autodiscovery) | Subscribers can follow; no point delaying |
| 4 | OpenGraph cards (no image) | Makes sharing look professional |
| 5 | Tags | Enables tag pages, related posts, tag feeds |
| 6 | Custom 404 | Small, personality-adding |
| 7 | Syntax highlighting | Biggest visual upgrade, but `syntect` adds weight |
| 8 | Dark mode | CSS refactor, philosophical choice on JS toggle |
| 9 | Search | Most complex, but htmx approach is clean |
| 10 | Sitemap | Only if you want search engines |

---

## What This Looks Like After

A visitor comes to the blog. They see a clean list of posts, each with a
reading time estimate and tags. They click into a post — syntax-highlighted
code, a table of contents at the top, related posts at the bottom, and
previous/next arrows. They share the post on Discord and it unfurls with a
title, description, and (eventually) a nice card image. They subscribe to
the Atom feed in their reader. They search for a topic and get instant
results. They flip to dark mode when reading at night.

None of this required a database, an external service, or a change to the
deployment model. The server is still a single statically-linked binary
sandboxed with Landlock, serving pre-rendered pages from memory. It just
serves richer pages.

---

## What This Deliberately Excludes

- **Comments or any user-generated content** — no inbox, no database, no
  moderation. Out of scope.
- **Newsletter / email** — no Mailgun, no subscriber list. Out of scope.
- **Analytics** — no counts, no tracking, not even privacy-friendly
  analytics. Can revisit later if desired.
- **Admin panel / editing UI** — posts are markdown files, edited locally,
  deployed via the existing pipeline. Out of scope.
- **Persistent state of any kind** — the server remains a memory appliance.
