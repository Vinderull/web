pub mod config;
pub mod feed;
pub mod posts;
pub mod sandbox;
mod templates;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};

use askama::Template;
use serde::Deserialize;
use templates::{
    IndexTemplate, NotFoundTemplate, PageTemplate, PostTemplate, SearchResultsTemplate,
    TagTemplate, TagsIndexTemplate,
};

/// Site name shown in the header and every page title. Change it here to
/// rename the site everywhere at once.
pub const SITE_NAME: &str = "My Bloginorium";

/// Public URL of the site. Used for Atom feed absolute URIs. Change this
/// to match your domain before deploying.
pub const SITE_URL: &str = "https://bloginorium.me";

/// Site author shown in the Atom feed. RFC 4287 requires at least one
/// author element on the feed.
pub const SITE_AUTHOR: &str = "Ryan Dufour";

/// Content-Security-Policy header value applied to every HTML response.
///
/// Least privilege: `default-src 'none'` denies every resource class unless
/// explicitly allowed. Only same-origin scripts (the `/static/js/htmx.min.js`
/// bundle; no inline scripts are used), styles, images (incl. the SVG
/// favicon), and HTMX fetch/connect traffic are permitted. `base-uri 'none'`
/// blocks `<base>` injection, `form-action 'none'` blocks form submission,
/// and `frame-ancestors 'none'` forbids framing.
pub const CSP_VALUE: &str = "default-src 'none'; base-uri 'none'; connect-src 'self'; form-action 'none'; frame-ancestors 'none'; img-src 'self'; script-src 'self'; style-src 'self'";

/// Permissions-Policy header value applied to every HTML response.
/// The site uses none of these browser capabilities, so they are denied to
/// every origin.
pub const PERMISSIONS_POLICY_VALUE: &str = "accelerometer=(), ambient-light-sensor=(), autoplay=(), battery=(), camera=(), display-capture=(), document-domain=(), fullscreen=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), picture-in-picture=(), screen-wake-lock=(), usb=(), web-share=(), xr-spatial-tracking=()";

// The three static assets are embedded into the binary at compile time, so the
// request path never touches the filesystem: `include_bytes!` bakes the exact
// vendored source bytes in, and each route serves them from RAM with a static,
// correct content type. htmx's `.min.js` bytes are thus the exact SRI-pinned
// release from scripts/update-htmx.sh, byte for byte.
const STATIC_CSS: &[u8] = include_bytes!("../static/css/main.css");
const STATIC_JS: &[u8] = include_bytes!("../static/js/htmx.min.js");
const STATIC_FAVICON: &[u8] = include_bytes!("../static/favicon.svg");

#[derive(Clone)]
struct AppState {
    index_html: Bytes,
    index_etag: HeaderValue,
    // Pre-rendered post pages keyed by slug. Posts are immutable for the
    // process lifetime, so each page renders once at boot and is served as a
    // pure lookup + header check.
    // note: flat HashMap; fine while posts fit in RAM. A DB-backed store
    // would replace this at corpus scale.
    post_pages: Arc<HashMap<String, (Bytes, HeaderValue)>>,
    // The in-memory posts, retained for the read-only `/search` and `/tags/*`
    // routes. Immutable after boot.
    posts: Arc<Vec<posts::Post>>,
    // Pre-rendered `/tags` index (never empty).
    tags_index_html: Bytes,
    tags_index_etag: HeaderValue,
    // Pre-rendered `/tags/{tag}` post lists.
    tag_pages: Arc<HashMap<String, (Bytes, HeaderValue)>>,
    // Pre-rendered `/about` page (present only if `content/pages/about.md`
    // exists). Served like any other cached HTML page.
    about_page: Option<(Bytes, HeaderValue)>,
    // Pre-rendered 404 page, served on any unknown path or unknown slug/tag.
    not_found_html: Bytes,
    // Per-post lowercased search haystack, index-aligned with `posts`. Built
    // once at boot so every lookup during a search avoids re-lowercasing the
    // whole corpus per keystroke.
    // note: linear scan over the corpus per request; fine while posts fit
    // in RAM, an inverted index would replace this at corpus scale.
    search_haystacks: Arc<Vec<String>>,
    // Pre-rendered Atom feed. Built once at boot, served from memory like
    // every other page.
    feed_xml: Bytes,
    feed_etag: HeaderValue,
}

/// Reject every method but GET and HEAD before routing.
///
/// The router only serves GET (plus HEAD via Axum's automatic GET handling),
/// so any other method gets a bodyless 405 with an explicit `Allow` header
/// instead of reaching a route or the 404 fallback.
async fn method_guard(req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD) {
        next.run(req).await
    } else {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response()
    }
}

/// Build the application router from loaded posts.
///
/// Pre-renders the index, every post page, the tag index, and every tag page
/// once, so the request path is a pure lookup + header check with no
/// per-request rendering. Posts are retained in `Arc` for the read-only
/// `/search` and `/tags/*` routes. The three static assets (CSS, htmx JS,
/// favicon) are embedded into the binary at compile time.
pub fn build_app(
    posts: Vec<posts::Post>,
    about_page: Option<posts::Page>,
) -> anyhow::Result<Router> {
    let posts = Arc::new(posts);
    let all_posts: Vec<&posts::Post> = posts.iter().collect();
    let index_html = IndexTemplate {
        posts: &all_posts,
        query: "",
        site_name: SITE_NAME,
    }
    .render()
    .context("rendering index template")?;
    let index_etag = etag_for(index_html.as_bytes());
    let index_html = Bytes::from(index_html);

    let not_found_html = Bytes::from(
        NotFoundTemplate {
            site_name: SITE_NAME,
        }
        .render()
        .context("rendering 404 template")?,
    );

    let mut post_pages = HashMap::with_capacity(posts.len());
    let posts_slice = posts.as_slice();
    for (i, p) in posts_slice.iter().enumerate() {
        let newer = (i > 0).then(|| &posts_slice[i - 1]);
        let older = (i + 1 < posts_slice.len()).then(|| &posts_slice[i + 1]);
        let html = PostTemplate {
            post: p,
            newer,
            older,
            site_name: SITE_NAME,
        }
        .render()
        .with_context(|| format!("rendering post {}", p.slug))?;
        let etag = etag_for(html.as_bytes());
        post_pages.insert(p.slug.clone(), (Bytes::from(html), etag));
    }
    let post_pages = Arc::new(post_pages);

    // Precompute a lowercased search haystack per post, index-aligned with
    // `posts`, so search never re-allocates or re-lowercases the corpus on the
    // request path.
    let search_haystacks = Arc::new(build_haystacks(posts_slice));

    // Pre-render the tag index and one page per tag (sorted alphabetically).
    let tags = collect_tags(&posts);
    let tags_index_html = Bytes::from(
        TagsIndexTemplate {
            tags: &tags,
            site_name: SITE_NAME,
        }
        .render()
        .context("rendering tags index")?,
    );
    let tags_index_etag = etag_for(&tags_index_html);
    let mut tag_pages = HashMap::with_capacity(tags.len());
    for (tag, _) in &tags {
        let list: Vec<&posts::Post> = posts
            .iter()
            .filter(|p| p.tags.iter().any(|t| t == tag))
            .collect();
        let html = TagTemplate {
            tag,
            posts: &list,
            site_name: SITE_NAME,
        }
        .render()
        .with_context(|| format!("rendering tag {tag}"))?;
        let etag = etag_for(html.as_bytes());
        tag_pages.insert(tag.clone(), (Bytes::from(html), etag));
    }
    let tag_pages = Arc::new(tag_pages);

    // Pre-render the standalone about page. It was loaded from disk before
    // sandboxing; the request path performs no filesystem access.
    // Its absence is not an error — the `/about` route just 404s.
    let about_page = about_page
        .map(|page| {
            let html = PageTemplate {
                page: &page,
                site_name: SITE_NAME,
            }
            .render()
            .context("rendering about page")?;
            let etag = etag_for(html.as_bytes());
            Ok::<_, anyhow::Error>((Bytes::from(html), etag))
        })
        .transpose()?;

    // Pre-render the Atom feed from all posts. Rendered once at boot, served
    // from memory like every other page.
    let feed_xml = Bytes::from(feed::build_feed(posts_slice));
    let feed_etag = etag_for(&feed_xml);

    let state = AppState {
        index_html,
        index_etag,
        post_pages,
        posts,
        tags_index_html,
        tags_index_etag,
        tag_pages,
        about_page,
        not_found_html,
        search_haystacks,
        feed_xml,
        feed_etag,
    };

    Ok(Router::new()
        .route("/", get(index))
        .route("/posts/{slug}", get(post))
        .route("/search", get(search))
        .route("/tags", get(tags_index))
        .route("/tags/{tag}", get(tag))
        .route("/about", get(about))
        .route("/teapot", get(teapot))
        .route("/feed.xml", get(feed))
        .route("/feed", get(|| async { Redirect::permanent("/feed.xml") }))
        .route("/robots.txt", get(robots_txt))
        .route("/healthz", get(|| async { "ok" }))
        // The three embedded assets: exact routes, so any other `/static/*`
        // path falls through to the 404 fallback.
        .route("/static/css/main.css", get(static_css))
        .route("/static/js/htmx.min.js", get(static_js))
        .route("/static/favicon.svg", get(static_favicon))
        .fallback(not_found)
        .with_state(state)
        .layer(middleware::from_fn(method_guard)))
}

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    cached_html(headers, state.index_html.clone(), state.index_etag.clone())
}

async fn post(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    match state.post_pages.get(&slug).cloned() {
        Some((html, etag)) => cached_html(headers, html, etag),
        None => not_found_response(&state),
    }
}

async fn about(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.about_page.clone() {
        Some((html, etag)) => cached_html(headers, html, etag),
        None => not_found_response(&state),
    }
}

async fn teapot() -> Response {
    // Easter egg. Short-circuits before any real route; deliberately no
    // caching/state so it stays zero-cost and invisible to real traffic.
    let mut resp = Response::new(Body::from(
        "418 I'm a teapot:\n\nThe requested entity body is short and stout.\nTip me over and pour me out.\n",
    ));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    apply_security_headers(&mut resp);
    (StatusCode::IM_A_TEAPOT, resp).into_response()
}

async fn robots_txt() -> Response {
    let mut resp = Response::new(Body::from(
        "User-agent: *\nDisallow: /search\nDisallow: /teapot\n",
    ));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    apply_security_headers(&mut resp);
    resp
}

/// Serve the embedded `main.css` with a fixed stylesheet content type.
///
/// GET and HEAD both succeed (axum auto-handles HEAD on GET routes, leaving
/// the body empty); any other `/static/*` path misses these exact routes and
/// reaches the 404 fallback.
async fn static_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        Bytes::from_static(STATIC_CSS),
    )
        .into_response()
}

/// Serve the embedded vendored `htmx.min.js` with a browser-compatible
/// JavaScript content type.
async fn static_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        Bytes::from_static(STATIC_JS),
    )
        .into_response()
}

/// Serve the embedded SVG favicon.
async fn static_favicon() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        Bytes::from_static(STATIC_FAVICON),
    )
        .into_response()
}

async fn feed(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if etag_matches(&headers, &state.feed_etag) {
        // 304 mirrors the selected representation's metadata (ETag +
        // Cache-Control) with an empty body and no Content-Type.
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=43200, s-maxage=43200, immutable"),
        );
        resp.headers_mut()
            .insert(header::ETAG, state.feed_etag.clone());
        return resp;
    }
    let mut resp = Response::new(Body::from(state.feed_xml.clone()));
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=43200, s-maxage=43200, immutable"),
    );
    resp.headers_mut()
        .insert(header::ETAG, state.feed_etag.clone());
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/atom+xml; charset=utf-8"),
    );
    resp
}

async fn tags_index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    cached_html(
        headers,
        state.tags_index_html.clone(),
        state.tags_index_etag.clone(),
    )
}

async fn tag(
    State(state): State<AppState>,
    AxumPath(tag): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    match state.tag_pages.get(&tag).cloned() {
        Some((html, etag)) => cached_html(headers, html, etag),
        None => not_found_response(&state),
    }
}

/// Fallback for any path that matches no route.
async fn not_found(State(state): State<AppState>) -> Response {
    not_found_response(&state)
}

/// Build the shared custom 404 response: the pre-rendered page with a 404
/// status and no caching (a 404 should never be cached).
fn not_found_response(state: &AppState) -> Response {
    let mut resp = Response::new(Body::from(state.not_found_html.clone()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_security_headers(&mut resp);
    (StatusCode::NOT_FOUND, resp).into_response()
}

/// Collect the sorted set of tags with per-tag post counts.
fn collect_tags(posts: &[posts::Post]) -> Vec<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in posts {
        for t in &p.tags {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut tags: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(t, n)| (t.to_string(), n))
        .collect();
    tags.sort_by(|a, b| a.0.cmp(&b.0));
    tags
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

/// Upper bound on accepted query characters. Truncation (not rejection) keeps
/// a too-long `q` harmless while still returning useful results and bounding
/// the per-request allocation cost of lowercasing/matching.
const MAX_QUERY_LEN: usize = 200;

/// Server-side search over the in-memory posts.
///
/// htmx requests (detected via the `HX-Request: true` header) get the bare
/// `<ul id="post-list">` fragment to swap into the index's list. A plain
/// navigation — Lynx, curl, JS disabled — gets a full document (the index
/// template re-rendered with the filtered posts) so search works with no JS.
///
/// The body is deliberately not cached so re-searching always sees fresh
/// results.
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SearchQuery>,
) -> Response {
    let raw = params.q.unwrap_or_default();
    // Truncate once up front: the truncated form feeds both the search
    // function and the template, so the reflected query in the response
    // body is bounded regardless of input length.
    let query = truncate_query(&raw);
    let results = search_posts_with(&state.posts, &state.search_haystacks, &query);
    let is_htmx = headers
        .get(header::HeaderName::from_static("hx-request"))
        .is_some_and(|v| v == "true");
    let body = if is_htmx {
        SearchResultsTemplate {
            posts: &results,
            query: &query,
        }
        .render()
        .unwrap_or_default()
    } else {
        IndexTemplate {
            posts: &results,
            query: &query,
            site_name: SITE_NAME,
        }
        .render()
        .unwrap_or_default()
    };
    let mut resp = Response::new(Body::from(body));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_security_headers(&mut resp);
    resp
}

/// Truncate and trim a search query, returning the empty string when the
/// result is blank — a blank query means "return all posts."
fn truncate_query(raw: &str) -> String {
    // Cut to the first MAX_QUERY_LEN chars on a char boundary, trim the
    // borrowed slice, then do the single to_owned allocation.
    let end = raw
        .char_indices()
        .nth(MAX_QUERY_LEN)
        .map_or(raw.len(), |(i, _)| i);
    raw[..end].trim().to_string()
}

/// Build the per-post lowercased search haystack (title + description + body),
/// index-aligned with `posts`. Built once at boot and reused for every search.
fn build_haystacks(posts: &[posts::Post]) -> Vec<String> {
    posts
        .iter()
        .map(|p| {
            let mut h = p.title.to_lowercase();
            if let Some(d) = &p.description {
                h.push('\n');
                h.push_str(&d.to_lowercase());
            }
            h.push('\n');
            h.push_str(&p.html.to_lowercase());
            h
        })
        .collect()
}

/// Case-insensitive substring match on the pre-lowercased title/description/
/// body haystack. `haystacks` is index-aligned with `posts` and built once at
/// boot. The query is lowercased internally, so any case matches. A blank
/// query returns every post. The caller is responsible for truncation and
/// trimming; the query here is already a trimmed, length-bounded string.
fn search_posts_with<'a>(
    posts: &'a [posts::Post],
    haystacks: &[String],
    query: &str,
) -> Vec<&'a posts::Post> {
    if query.is_empty() {
        return posts.iter().collect();
    }
    let q = query.to_lowercase();
    posts
        .iter()
        .zip(haystacks)
        .filter(|(_, h)| h.contains(&q))
        .map(|(p, _)| p)
        .collect()
}

/// True when any ETag in the request's `If-None-Match` header matches `etag`.
///
/// Follows RFC 9110 §13.1.2 GET/HEAD semantics: `*` matches any current
/// representation (every caller passes the ETag of an existing one), and
/// entity-tags are compared weakly, so `W/"tag"` equals `"tag"`. Members are
/// parsed conservatively — a malformed token never matches. Allocation-free.
fn etag_matches(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    let Some(server_opaque) = etag.to_str().ok().and_then(opaque_of) else {
        return false;
    };
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|inm| {
            inm.split(',').any(|member| {
                let member = member.trim();
                // `*` is "any current representation" (RFC 9110 §13.1.2); the
                // header only exists because a representation does.
                member == "*" || opaque_of(member) == Some(server_opaque)
            })
        })
}

/// The opaque-tag of a well-formed entity-tag (`"…"` or `W/"…"`), or `None`
/// when the tag is malformed. Dropping the `W/` prefix is exactly the weak
/// comparison RFC 9110 §8.8.3.2 requires for `If-None-Match`.
fn opaque_of(tag: &str) -> Option<&str> {
    let body = tag.strip_prefix("W/").unwrap_or(tag);
    let opaque = body.strip_prefix('"')?.strip_suffix('"')?;
    opaque.bytes().all(is_etagc).then_some(opaque)
}

/// One `etagc` character (RFC 9110 §8.8.3): `!`, `#`–`~`, or obs-text.
/// Quotes, spaces, controls, and DEL are excluded, so an embedded quote or
/// space marks the tag malformed rather than a near-match.
fn is_etagc(b: u8) -> bool {
    b == 0x21 || (0x23..=0x7E).contains(&b) || b >= 0x80
}

/// Build a weakly-caching HTML response: 304 on matching `If-None-Match`,
/// otherwise the body with `Cache-Control` + `ETag` headers.
fn cached_html(headers: HeaderMap, body: Bytes, etag: HeaderValue) -> Response {
    if etag_matches(&headers, &etag) {
        // 304 mirrors the selected representation's metadata (ETag +
        // Cache-Control) with an empty body and no Content-Type.
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=43200, s-maxage=43200, immutable"),
        );
        resp.headers_mut().insert(header::ETAG, etag);
        return resp;
    }
    let mut resp = Response::new(Body::from(body));
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=43200, s-maxage=43200, immutable"),
    );
    resp.headers_mut().insert(header::ETAG, etag);
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    apply_security_headers(&mut resp);
    resp
}

/// Apply the hardening headers that are safe on every HTML response:
/// MIME sniffing off, framing denied, a strict referrer policy, and the
/// site's CSP. Shared by cached pages and the uncached `/search` fragment.
fn apply_security_headers(resp: &mut Response) {
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp.headers_mut()
        .insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    resp.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP_VALUE),
    );
    resp.headers_mut().insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY_VALUE),
    );
}

/// Stable ETag for a rendered body.
///
/// xxh3 is deterministic across processes and Rust versions, so the same
/// content yields the same ETag on every boot — a client that cached a page
/// before a deploy keeps getting a 304 instead of a full re-download.
fn etag_for(s: &[u8]) -> HeaderValue {
    let digest = xxhash_rust::xxh3::xxh3_64(s);
    HeaderValue::from_str(&format!("\"{digest:x}\"")).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn req_with_inm(etag: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(e) = etag {
            h.insert(header::IF_NONE_MATCH, HeaderValue::from_str(e).unwrap());
        }
        h
    }

    #[test]
    fn cached_html_200_with_headers_when_no_inm() {
        let resp = cached_html(
            req_with_inm(None),
            Bytes::from_static(b"hello"),
            HeaderValue::from_static("\"abc\""),
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::ETAG).unwrap(), "\"abc\"");
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=43200, s-maxage=43200, immutable"
        );
    }

    #[test]
    fn cached_html_304_on_matching_etag() {
        let resp = cached_html(
            req_with_inm(Some("\"abc\"")),
            Bytes::from_static(b"hello"),
            HeaderValue::from_static("\"abc\""),
        );
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        // 304 mirrors the cached metadata with an empty body: same ETag and
        // Cache-Control, no Content-Type.
        assert_eq!(resp.headers().get(header::ETAG).unwrap(), "\"abc\"");
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=43200, s-maxage=43200, immutable"
        );
        assert!(resp.headers().get(header::CONTENT_TYPE).is_none());
    }

    #[test]
    fn cached_html_200_on_nonmatching_etag() {
        let resp = cached_html(
            req_with_inm(Some("\"xyz\"")),
            Bytes::from_static(b"hello"),
            HeaderValue::from_static("\"abc\""),
        );
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn cached_html_304_on_csv_etag_list() {
        // If-None-Match may list several ETags; a match anywhere is a 304.
        let resp = cached_html(
            req_with_inm(Some("\"other\", \"abc\"")),
            Bytes::from_static(b"hello"),
            HeaderValue::from_static("\"abc\""),
        );
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn cached_html_sets_security_headers() {
        let resp = cached_html(
            req_with_inm(None),
            Bytes::from_static(b"hello"),
            HeaderValue::from_static("\"abc\""),
        );
        let h = resp.headers();
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert_eq!(h.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(
            h.get(header::REFERRER_POLICY).unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert_eq!(h.get(header::CONTENT_SECURITY_POLICY).unwrap(), CSP_VALUE,);
        assert_eq!(
            h.get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
    }

    #[test]
    fn etag_stable_for_same_input() {
        assert_eq!(etag_for(b"hello"), etag_for(b"hello"));
        assert_ne!(etag_for(b"hello"), etag_for(b"world"));
    }

    #[test]
    fn etag_matches_wildcard_and_weak_forms() {
        let strong = HeaderValue::from_static("\"abc\"");
        // `*` matches any current representation (RFC 9110 §13.1.2).
        assert!(etag_matches(&req_with_inm(Some("*")), &strong));
        // Weak comparison: W/"tag" equals "tag".
        assert!(etag_matches(&req_with_inm(Some("W/\"abc\"")), &strong));
        // Wildcard/weak members work inside a CSV list too.
        assert!(etag_matches(
            &req_with_inm(Some("\"other\", W/\"abc\"")),
            &strong
        ));
        assert!(etag_matches(&req_with_inm(Some("\"other\", *")), &strong));
    }

    #[test]
    fn etag_matches_rejects_nonmatching_and_malformed() {
        let strong = HeaderValue::from_static("\"abc\"");
        assert!(!etag_matches(&req_with_inm(Some("\"xyz\"")), &strong));
        // Substring and near-miss tokens are not matches.
        assert!(!etag_matches(&req_with_inm(Some("\"ab\"")), &strong));
        assert!(!etag_matches(&req_with_inm(Some("abc")), &strong));
        assert!(!etag_matches(&req_with_inm(Some("abc\"")), &strong));
        assert!(!etag_matches(&req_with_inm(Some("\"abc\"x")), &strong));
        assert!(!etag_matches(&req_with_inm(Some("W/abc")), &strong));
        // "W/" is case-sensitive (RFC 9110 §8.8.3.1); lowercase is malformed.
        assert!(!etag_matches(&req_with_inm(Some("w/\"abc\"")), &strong));
        // Embedded whitespace is not an etagc.
        assert!(!etag_matches(&req_with_inm(Some("\"ab c\"")), &strong));
        // Quoted "*" is an entity-tag, not the wildcard; "*x" is not "*".
        assert!(!etag_matches(&req_with_inm(Some("\"*\"")), &strong));
        assert!(!etag_matches(&req_with_inm(Some("*x")), &strong));
        // Empty header and absent header never match.
        assert!(!etag_matches(&req_with_inm(Some("")), &strong));
        assert!(!etag_matches(&req_with_inm(None), &strong));
    }

    #[test]
    fn truncate_query_caps_at_char_boundary() {
        // The cap counts chars, not bytes: a multi-byte cut must land on a
        // char boundary and keep exactly MAX_QUERY_LEN chars.
        let wide = "é".repeat(MAX_QUERY_LEN + 3);
        let capped = truncate_query(&wide);
        assert_eq!(capped.chars().count(), MAX_QUERY_LEN);
        assert_eq!(capped, "é".repeat(MAX_QUERY_LEN));
        // Exactly MAX_QUERY_LEN chars pass through whole.
        let exact = "x".repeat(MAX_QUERY_LEN);
        assert_eq!(truncate_query(&exact), exact);
        // Trimming still happens after capping.
        assert_eq!(
            truncate_query(&format!("  {}", "y".repeat(MAX_QUERY_LEN))),
            "y".repeat(MAX_QUERY_LEN - 2)
        );
        assert_eq!(truncate_query("   "), "");
    }

    #[test]
    fn etag_is_a_fixed_golden_digest() {
        // Pins the exact xxh3 digest so cross-process/cross-version stability
        // is caught here rather than silently drifting. If the hasher or its
        // parameters ever change, this test fails loudly.
        assert_eq!(etag_for(b"hello").to_str().unwrap(), "\"9555e8555c62dcfd\"");
    }

    // The post route is now a pure lookup against pre-rendered pages; this
    // mirrors the boot pipeline and guards against regressions there.
    fn build_post_pages(posts: &[posts::Post]) -> HashMap<String, (Bytes, HeaderValue)> {
        let mut map = HashMap::new();
        for (i, p) in posts.iter().enumerate() {
            let newer = (i > 0).then(|| &posts[i - 1]);
            let older = (i + 1 < posts.len()).then(|| &posts[i + 1]);
            let html = PostTemplate {
                post: p,
                newer,
                older,
                site_name: SITE_NAME,
            }
            .render()
            .unwrap();
            let etag = etag_for(html.as_bytes());
            map.insert(p.slug.clone(), (Bytes::from(html), etag));
        }
        map
    }

    fn fake_post(slug: &str) -> posts::Post {
        posts::Post {
            slug: slug.to_string(),
            title: slug.to_string(),
            date: "2024-01-01".to_string(),
            date_display: "January 1, 2024".to_string(),
            description: None,
            tags: Vec::new(),
            html: String::new(),
            toc: String::new(),
            reading_time: 1,
        }
    }

    #[test]
    fn post_nav_renders_prev_and_next() {
        // Sorted newest-first, so [0] is newer than [1].
        let posts = vec![
            fake_post("newest"),
            fake_post("middle"),
            fake_post("oldest"),
        ];
        let pages = build_post_pages(&posts);

        let newest = String::from_utf8(pages["newest"].0.to_vec()).unwrap();
        assert!(
            newest.contains("middle"),
            "newest should link only to older"
        );
        assert!(!newest.contains("/posts/oldest"));

        let middle = String::from_utf8(pages["middle"].0.to_vec()).unwrap();
        assert!(
            middle.contains("/posts/newest"),
            "middle should link to newer"
        );
        assert!(
            middle.contains("/posts/oldest"),
            "middle should link to older"
        );

        let oldest = String::from_utf8(pages["oldest"].0.to_vec()).unwrap();
        assert!(
            oldest.contains("/posts/middle"),
            "oldest should link only to newer"
        );
        assert!(!oldest.contains("/posts/newest"));
    }

    #[test]
    fn single_post_has_no_nav() {
        let posts = vec![fake_post("only")];
        let pages = build_post_pages(&posts);
        let html = String::from_utf8(pages["only"].0.to_vec()).unwrap();
        assert!(!html.contains("post-nav"), "single post should have no nav");
    }

    #[test]
    fn collect_tags_sorts_and_counts() {
        let mut posts = vec![fake_post("a"), fake_post("b"), fake_post("c")];
        posts[0].tags = vec!["rust".into(), "web".into()];
        posts[1].tags = vec!["rust".into()];
        let tags = collect_tags(&posts);
        assert_eq!(
            tags,
            vec![("rust".to_string(), 2), ("web".to_string(), 1),],
            "tags sorted alphabetically with per-tag post counts"
        );
    }

    #[test]
    fn post_pages_lookup_round_trip() {
        let posts = posts::load_all(std::path::Path::new("content")).unwrap();
        if posts.is_empty() {
            return;
        }
        let pages = build_post_pages(&posts);
        for p in &posts {
            let (html, etag) = pages
                .get(&p.slug)
                .unwrap_or_else(|| panic!("missing pre-rendered page for slug {}", p.slug));
            assert!(!html.is_empty(), "empty body for {}", p.slug);
            // ETag is a quoted opaque token.
            let etag = etag.to_str().unwrap();
            assert!(etag.starts_with('"') && etag.ends_with('"'));
            assert_eq!(etag, etag_for(html.as_ref()).to_str().unwrap());
        }
    }

    #[test]
    fn search_matches_title_description_and_body_case_insensitively() {
        let posts = posts::load_all(std::path::Path::new("content")).unwrap();
        if posts.is_empty() {
            return;
        }
        let haystacks = build_haystacks(&posts);
        // Every post must match an empty query.
        assert_eq!(search_posts_with(&posts, &haystacks, "").len(), posts.len());
        assert_eq!(search_posts_with(&posts, &haystacks, "").len(), posts.len());

        // A title match, including a case-insensitive variant.
        let by_title = search_posts_with(&posts, &haystacks, &posts[0].title);
        assert!(
            by_title.iter().any(|p| p.slug == posts[0].slug),
            "title should match its own post"
        );
        assert!(
            search_posts_with(&posts, &haystacks, &posts[0].title.to_uppercase())
                .iter()
                .any(|p| p.slug == posts[0].slug),
            "title match should be case-insensitive"
        );

        // A description-only match.
        if let Some(desc) = &posts[0].description {
            assert!(
                search_posts_with(&posts, &haystacks, desc)
                    .iter()
                    .any(|p| p.slug == posts[0].slug),
                "description should match its own post"
            );
        }

        // A garbage query returns nothing.
        assert!(search_posts_with(&posts, &haystacks, "qqqqzzzznope").is_empty());
    }

    #[test]
    fn search_caps_oversized_query() {
        let posts = vec![
            posts::Post {
                slug: "x".into(),
                title: "abc".into(),
                date: "2024-01-01".into(),
                date_display: "January 1, 2024".into(),
                description: None,
                tags: Vec::new(),
                html: "abc".into(),
                toc: String::new(),
                reading_time: 1,
            },
            fake_post("other"),
        ];
        let haystacks = build_haystacks(&posts);
        // A short, real query matches the post whose haystack contains it.
        assert!(
            search_posts_with(&posts, &haystacks, "abc")
                .iter()
                .any(|p| p.slug == "x")
        );
        // An oversized query is truncated by truncate_query before reaching
        // search_posts_with. After truncation to MAX_QUERY_LEN, the trailing
        // "abc" is dropped so nothing matches.
        let oversized = format!("{}abc", "a".repeat(MAX_QUERY_LEN));
        assert!(oversized.len() > MAX_QUERY_LEN);
        let capped = truncate_query(&oversized);
        assert_eq!(
            search_posts_with(&posts, &haystacks, &capped).len(),
            0,
            "after truncation to MAX_QUERY_LEN the tail 'abc' is dropped"
        );
    }

    // Unknown paths and unknown slugs/tags all yield the custom 404 page.
    async fn not_found_body(uri: &str) -> (StatusCode, String) {
        let app = build_app(Vec::new(), None).unwrap();
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn unknown_path_returns_custom_404() {
        let (status, body) = not_found_body("/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("404"), "should render the custom 404 page");
    }

    #[tokio::test]
    async fn unknown_slug_returns_custom_404() {
        let (status, body) = not_found_body("/posts/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("404"), "should render the custom 404 page");
    }

    #[tokio::test]
    async fn missing_about_returns_custom_404() {
        let (status, body) = not_found_body("/about").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("404"), "should render the custom 404 page");
    }

    #[tokio::test]
    async fn robots_txt_disallows_search() {
        let app = build_app(Vec::new(), None).unwrap();
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(
            body,
            "User-agent: *\nDisallow: /search\nDisallow: /teapot\n"
        );
    }

    #[tokio::test]
    async fn feed_304_echoes_etag_and_cache_control() {
        let app = build_app(Vec::new(), None).unwrap();
        let first = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/feed.xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let etag = first
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/feed.xml")
                    .header(header::IF_NONE_MATCH, etag.as_str())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=43200, s-maxage=43200, immutable"
        );
        assert_eq!(
            resp.headers().get(header::ETAG).unwrap().to_str().unwrap(),
            etag.as_str(),
            "304 must echo the matching ETag"
        );
        assert!(
            resp.headers().get(header::CONTENT_TYPE).is_none(),
            "304 must not carry Content-Type"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "304 must have no body");
    }
}
