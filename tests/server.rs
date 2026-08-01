use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;
use web::{build_app, posts};

/// Wire up the real app against the repo's content/ and static/ dirs.
fn app() -> Router {
    let posts = posts::load_all(Path::new("content")).expect("load posts");
    build_app(posts, Path::new("static")).expect("build app")
}

/// Drive a request through the router in-process (no TCP socket) and return
/// status, headers, and collected body bytes.
async fn req(
    app: Router,
    method: &str,
    uri: &str,
    inm: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(e) = inm {
        builder = builder.header(header::IF_NONE_MATCH, e);
    }
    let resp = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, bytes.to_vec())
}

#[tokio::test]
async fn index_returns_200_with_cache_headers() {
    let (status, headers, body) = req(app(), "GET", "/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=60, s-maxage=600"
    );
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("<h1>"), "index should render base layout");
    assert!(body.contains("Posts"), "index should list posts");
}

#[tokio::test]
async fn index_304_on_matching_etag() {
    let (_, headers, _) = req(app(), "GET", "/", None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/", Some(etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}

#[tokio::test]
async fn post_page_200_for_known_slug() {
    let posts = posts::load_all(Path::new("content")).expect("load posts");
    let slug = match posts.first() {
        Some(p) => &p.slug,
        None => return, // no content to test against
    };
    let (status, headers, body) = req(app(), "GET", &format!("/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("<article>"),
        "post should render article wrapper"
    );
    // hello-world.md has h2 headings, so the precomputed ToC nav should render.
    if posts.iter().any(|p| p.slug == *slug && !p.toc.is_empty()) {
        assert!(
            body.contains("Table of Contents") && body.contains("class=\"toc\""),
            "post page should include the ToC nav"
        );
    }
}

#[tokio::test]
async fn post_404_for_unknown_slug() {
    let (status, _, _) = req(app(), "GET", "/posts/does-not-exist", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (status, _, body) = req(app(), "GET", "/healthz", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
}

#[tokio::test]
async fn static_asset_served() {
    let (status, _, body) = req(app(), "GET", "/static/css/main.css", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.is_empty(), "css should have content");
}

#[tokio::test]
async fn post_page_304_on_matching_etag() {
    let posts = posts::load_all(Path::new("content")).expect("load posts");
    let slug = match posts.first() {
        Some(p) => &p.slug,
        None => return,
    };
    let app = app();
    let uri = format!("/posts/{slug}");
    let (status, headers, _) = req(app.clone(), "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app, "GET", &uri, Some(etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}

#[tokio::test]
async fn search_returns_matching_posts() {
    let (status, headers, body) = req(app(), "GET", "/search?q=hello", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store",
        "search responses must not be cached"
    );
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("<ul"), "should render a list fragment");
    assert!(
        body.contains("/posts/hello-world"),
        "should include the matching post"
    );
}

#[tokio::test]
async fn search_with_empty_query_returns_all_posts() {
    let posts = posts::load_all(Path::new("content")).unwrap();
    if posts.is_empty() {
        return; // no content to test against
    }
    let (status, _, body) = req(app(), "GET", "/search?q=", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    for p in &posts {
        assert!(
            body.contains(&format!("/posts/{}", p.slug)),
            "empty query should include every post ({})",
            p.slug
        );
    }
}

#[tokio::test]
async fn search_with_no_match_returns_empty_state() {
    let (status, _, body) = req(app(), "GET", "/search?q=zzzznomatch", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("No posts match"),
        "should render the empty state"
    );
}
