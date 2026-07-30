mod config;
mod posts;
mod sandbox;
mod templates;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use anyhow::Context;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use askama::Template;
use config::Config;
use templates::{IndexTemplate, PostTemplate};

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
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!("Content dir: {}", config.content_dir.display());
    tracing::info!("Static dir: {}", config.static_dir.display());

    // Load posts from disk before sandboxing
    let loaded_posts = posts::load_all(&config.content_dir)?;
    tracing::info!("Loaded {} posts", loaded_posts.len());

    // Pre-render the index page once at boot: posts are immutable for the
    // process lifetime, so the rendered HTML never changes either.
    let index_html = IndexTemplate {
        posts: &loaded_posts,
    }
    .render()
    .context("rendering index template")?;
    let index_etag = etag_for(index_html.as_bytes());
    let index_html = Bytes::from(index_html);

    // Pre-render every post page once at boot, same rationale as the index.
    let mut post_pages = HashMap::with_capacity(loaded_posts.len());
    for p in &loaded_posts {
        let html = PostTemplate { post: p }
            .render()
            .with_context(|| format!("rendering post {}", p.slug))?;
        let etag = etag_for(html.as_bytes());
        post_pages.insert(p.slug.clone(), (Bytes::from(html), etag));
    }
    let post_pages = Arc::new(post_pages);

    // Drop the raw post data now that pre-rendering is done. Each Post's html
    // body is already baked into the corresponding post_pages entry, and no
    // route reads the raw title/date/description fields. Keeping the Vec
    // alive (which it would, since block_on holds this frame until shutdown)
    // roughly doubles HTML memory for no benefit.
    // If a future route needs raw Post fields (e.g. a JSON API exposing
    // title/date/description), retain the Vec (or a slimmed struct) here.
    std::mem::drop(loaded_posts);

    // Bind listener before applying landlock (landlock would block bind on V4+)
    let listener = std::net::TcpListener::bind(&config.bind_addr)?;
    listener.set_nonblocking(true)?;
    tracing::info!("Listening on {}", config.bind_addr);

    // Apply landlock sandbox before starting tokio runtime so worker threads
    // inherit the landlock domain.
    sandbox::apply(&config.static_dir)?;

    // Create runtime after sandboxing so worker threads inherit restrictions
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let listener = tokio::net::TcpListener::from_std(listener)?;

        let state = AppState {
            index_html,
            index_etag,
            post_pages,
        };

        let app = Router::new()
            .route("/", get(index))
            .route("/posts/{slug}", get(post))
            .route("/healthz", get(|| async { "ok" }))
            .nest_service("/static", ServeDir::new(&config.static_dir))
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        let shutdown = async {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            let sigterm = async {
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("install SIGTERM handler")
                    .recv()
                    .await;
            };
            #[cfg(not(unix))]
            let sigterm = std::future::pending::<()>();

            tokio::select! {
                _ = ctrl_c => {}
                _ = sigterm => {}
            }
            tracing::info!("Shutdown signal received");
        };

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
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
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let (html, etag) = state
        .post_pages
        .get(&slug)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(cached_html(headers, html, etag))
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
        HeaderValue::from_static("public, max-age=300"),
    );
    resp.headers_mut().insert(header::ETAG, etag);
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
            "public, max-age=300"
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
}
