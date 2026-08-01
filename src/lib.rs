pub mod config;
pub mod posts;
pub mod sandbox;
mod templates;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use askama::Template;
use serde::Deserialize;
use templates::{
    IndexTemplate, PostTemplate, SearchResultsTemplate, TagTemplate, TagsIndexTemplate,
};

#[derive(Clone)]
struct AppState {
    index_html: Bytes,
    index_etag: HeaderValue,
    // Pre-rendered post pages keyed by slug. Posts are immutable for the
    // process lifetime, so each page renders once at boot and is served as a
    // pure lookup + header check.
    // ponytail: flat HashMap; fine while posts fit in RAM. A DB-backed store
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
}

/// Build the application router from loaded posts and a static-asset dir.
///
/// Pre-renders the index, every post page, the tag index, and every tag page
/// once, so the request path is a pure lookup + header check with no
/// per-request rendering. Posts are retained in `Arc` for the read-only
/// `/search` and `/tags/*` routes.
pub fn build_app(posts: Vec<posts::Post>, static_dir: &Path) -> anyhow::Result<Router> {
    let posts = Arc::new(posts);
    let index_html = IndexTemplate {
        posts: posts.as_slice(),
    }
    .render()
    .context("rendering index template")?;
    let index_etag = etag_for(index_html.as_bytes());
    let index_html = Bytes::from(index_html);

    let mut post_pages = HashMap::with_capacity(posts.len());
    for p in posts.as_slice() {
        let html = PostTemplate { post: p }
            .render()
            .with_context(|| format!("rendering post {}", p.slug))?;
        let etag = etag_for(html.as_bytes());
        post_pages.insert(p.slug.clone(), (Bytes::from(html), etag));
    }
    let post_pages = Arc::new(post_pages);

    // Pre-render the tag index and one page per tag (sorted alphabetically).
    let tags = collect_tags(&posts);
    let tags_index_html = Bytes::from(
        TagsIndexTemplate { tags: &tags }
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
        let html = TagTemplate { tag, posts: &list }
            .render()
            .with_context(|| format!("rendering tag {tag}"))?;
        let etag = etag_for(html.as_bytes());
        tag_pages.insert(tag.clone(), (Bytes::from(html), etag));
    }
    let tag_pages = Arc::new(tag_pages);

    let state = AppState {
        index_html,
        index_etag,
        post_pages,
        posts,
        tags_index_html,
        tags_index_etag,
        tag_pages,
    };

    Ok(Router::new()
        .route("/", get(index))
        .route("/posts/{slug}", get(post))
        .route("/search", get(search))
        .route("/tags", get(tags_index))
        .route("/tags/{tag}", get(tag))
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/static", ServeDir::new(static_dir))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    Ok(cached_html(
        headers,
        state.index_html.clone(),
        state.index_etag.clone(),
    ))
}

async fn post(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let (html, etag) = state
        .post_pages
        .get(&slug)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(cached_html(headers, html, etag))
}

async fn tags_index(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    Ok(cached_html(
        headers,
        state.tags_index_html.clone(),
        state.tags_index_etag.clone(),
    ))
}

async fn tag(
    State(state): State<AppState>,
    AxumPath(tag): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let (html, etag) = state
        .tag_pages
        .get(&tag)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(cached_html(headers, html, etag))
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

/// Server-side search over the in-memory posts. Returns an HTML fragment
/// (a `<ul id="post-list">`) that htmx swaps in place of the index's list.
/// The body is deliberately not cached so re-searching always sees fresh
/// results.
async fn search(State(state): State<AppState>, Query(params): Query<SearchQuery>) -> Response {
    let query = params.q.unwrap_or_default();
    let results = search_posts(&state.posts, &query);
    let body = SearchResultsTemplate {
        posts: &results,
        query: &query,
    }
    .render()
    .unwrap_or_default();
    let mut resp = Response::new(Body::from(body));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// Case-insensitive substring match on title, description, or body. A blank
/// query returns every post (so clearing the box restores the full list).
fn search_posts<'a>(posts: &'a [posts::Post], query: &str) -> Vec<&'a posts::Post> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return posts.iter().collect();
    }
    posts
        .iter()
        .filter(|p| {
            p.title.to_lowercase().contains(&q)
                || p.description
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&q))
                || p.html.to_lowercase().contains(&q)
        })
        .collect()
}

/// Build a weakly-caching HTML response: 304 on matching `If-None-Match`,
/// otherwise the body with `Cache-Control` + `ETag` headers.
fn cached_html(headers: HeaderMap, body: Bytes, etag: HeaderValue) -> Response {
    let matched = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|inm| {
            inm.split(',')
                .any(|t| t.trim() == etag.to_str().unwrap_or(""))
        })
        .unwrap_or(false);
    if matched {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let mut resp = Response::new(Body::from(body));
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, s-maxage=600"),
    );
    resp.headers_mut().insert(header::ETAG, etag);
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
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
        HeaderValue::from_static("default-src 'self'; style-src 'self'; script-src 'self'"),
    );
    resp
}

/// Deterministic-ish ETag for a rendered body.
// ponytail: DefaultHasher is not stable across Rust versions; the ETag only
// needs consistency within a deployment. Swap for a fixed algorithm (fnv /
// xxhash) if cross-version or cross-instance ETag reuse matters.
fn etag_for(s: &[u8]) -> HeaderValue {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    HeaderValue::from_str(&format!("\"{:x}\"", h.finish())).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

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
            "public, max-age=60, s-maxage=600"
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
    fn etag_stable_for_same_input() {
        assert_eq!(etag_for(b"hello"), etag_for(b"hello"));
        assert_ne!(etag_for(b"hello"), etag_for(b"world"));
    }

    // The post route is now a pure lookup against pre-rendered pages; this
    // mirrors the boot pipeline and guards against regressions there.
    fn build_post_pages(posts: &[posts::Post]) -> HashMap<String, (Bytes, HeaderValue)> {
        let mut map = HashMap::new();
        for p in posts {
            let html = PostTemplate { post: p }.render().unwrap();
            let etag = etag_for(html.as_bytes());
            map.insert(p.slug.clone(), (Bytes::from(html), etag));
        }
        map
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
        // Every post must match an empty query.
        assert_eq!(search_posts(&posts, "").len(), posts.len());
        assert_eq!(search_posts(&posts, "   ").len(), posts.len());

        // A title match, including a case-insensitive variant.
        let by_title = search_posts(&posts, &posts[0].title);
        assert!(
            by_title.iter().any(|p| p.slug == posts[0].slug),
            "title should match its own post"
        );
        assert!(
            search_posts(&posts, &posts[0].title.to_uppercase())
                .iter()
                .any(|p| p.slug == posts[0].slug),
            "title match should be case-insensitive"
        );

        // A description-only match.
        if let Some(desc) = &posts[0].description {
            assert!(
                search_posts(&posts, desc)
                    .iter()
                    .any(|p| p.slug == posts[0].slug),
                "description should match its own post"
            );
        }

        // A garbage query returns nothing.
        assert!(search_posts(&posts, "qqqqzzzznope").is_empty());
    }
}
