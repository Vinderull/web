use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;
use web::{build_app, posts};

/// A small fixed corpus so the tests don't depend on repo content/ posts.
fn test_corpus() -> Vec<posts::Post> {
    vec![
        posts::Post {
            slug: "hello-world".into(),
            title: "Hello World".into(),
            date: "2026-01-01".into(),
            date_display: "January 1, 2026".into(),
            description: Some("A first post".into()),
            tags: vec!["web".into()],
            // "cambodian" appears only here, isolating this post.
            html: "<p>about cambodian vodka and htmx</p>".into(),
            toc: String::new(),
            reading_time: 2,
        },
        posts::Post {
            slug: "second-post".into(),
            title: "Second Post".into(),
            date: "2026-01-02".into(),
            date_display: "January 2, 2026".into(),
            description: Some("all about rust and axum".into()),
            tags: vec!["rust".into()],
            html: "<p>all about rust and axum</p>".into(),
            toc: "<ul><li>Intro</li></ul>".into(),
            reading_time: 3,
        },
    ]
}

fn test_about() -> posts::Page {
    posts::Page {
        slug: "about".into(),
        title: "About".into(),
        html: "<p>Colophon goes here.</p>".into(),
    }
}

/// Wire up the app against the fixed synthetic corpus.
fn app() -> Router {
    build_app(test_corpus(), Some(test_about()), Path::new("static")).expect("build app")
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
        "public, max-age=43200, s-maxage=43200, immutable"
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
    let posts = test_corpus();
    let slug = match posts.first() {
        Some(p) => &p.slug,
        None => return, // empty corpus
    };
    let (status, headers, body) = req(app(), "GET", &format!("/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("<article>"),
        "post should render article wrapper"
    );
    // The "second-post" fixture has a ToC, so the precomputed nav should render.
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
async fn static_asset_missing_returns_404() {
    let (status, _, _) = req(app(), "GET", "/static/nope.css", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn index_sets_security_headers() {
    let (_, headers, _) = req(app(), "GET", "/", None).await;
    assert_eq!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
    assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(
        headers.get(header::REFERRER_POLICY).unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        web::CSP_VALUE,
    );
}

#[tokio::test]
async fn post_page_304_on_matching_etag() {
    let posts = test_corpus();
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

// "cambodian" appears only in the "hello-world" fixture's body, isolating it.
const QUERY: &str = "/search?q=cambodian";

async fn search_req(hx: bool, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if hx {
        builder = builder.header("HX-Request", "true");
    }
    let resp = app()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, bytes.to_vec())
}

#[tokio::test]
async fn search_returns_fragment_for_htmx() {
    let (status, headers, body) = search_req(true, QUERY).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store",
        "search responses must not be cached"
    );
    let body = String::from_utf8(body).unwrap();
    assert!(!body.contains("<html"), "htmx should get a bare fragment");
    assert!(
        body.contains("/posts/hello-world"),
        "should include the match"
    );
}

#[tokio::test]
async fn search_returns_full_document_for_plain_navigation() {
    let (status, _, body) = search_req(false, QUERY).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("<!DOCTYPE html>"),
        "plain navigation should be a full document (Lynx/curl/JS-off)"
    );
    assert!(
        body.contains("/posts/hello-world"),
        "should include the match"
    );
    // Only the matching post, not the full corpus.
    assert!(
        !body.contains("/posts/second-post"),
        "should filter results"
    );
}

#[tokio::test]
async fn search_sets_security_headers() {
    let (_, headers, _) = req(app(), "GET", "/search?q=hello", None).await;
    assert_eq!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
    assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(
        headers.get(header::REFERRER_POLICY).unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        web::CSP_VALUE,
    );
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
}

#[tokio::test]
async fn search_with_empty_query_returns_all_posts() {
    let posts = test_corpus();
    if posts.is_empty() {
        return; // empty corpus
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

#[tokio::test]
async fn tags_index_lists_all_tags() {
    let posts = test_corpus();
    let tags: Vec<String> = {
        let mut set = std::collections::BTreeSet::new();
        for p in &posts {
            set.extend(p.tags.iter().cloned());
        }
        set.into_iter().collect()
    };
    let (status, _, body) = req(app(), "GET", "/tags", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    for t in tags {
        assert!(
            body.contains(&format!("/tags/{t}")),
            "tags index should list tag {t}"
        );
    }
}

#[tokio::test]
async fn tag_page_lists_posts_for_tag() {
    let posts = test_corpus();
    let tag = match posts.iter().flat_map(|p| &p.tags).next() {
        Some(t) => t.clone(),
        None => return, // no tagged content
    };
    let (status, headers, body) = req(app(), "GET", &format!("/tags/{tag}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains(&format!("#{tag}")),
        "heading should name the tag"
    );
    for p in posts.iter().filter(|p| p.tags.contains(&tag)) {
        assert!(
            body.contains(&format!("/posts/{}", p.slug)),
            "tag page should list post {}",
            p.slug
        );
    }
}

#[tokio::test]
async fn tag_page_404_for_unknown_tag() {
    let (status, _, _) = req(app(), "GET", "/tags/does-not-exist", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_page_shows_tags() {
    let posts = test_corpus();
    let post = match posts.iter().find(|p| !p.tags.is_empty()) {
        Some(p) => p,
        None => return, // no tagged content
    };
    let (status, _, body) = req(app(), "GET", &format!("/posts/{}", post.slug), None).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    for t in &post.tags {
        assert!(
            body.contains(&format!("/tags/{t}")),
            "post page should link to tag {t}"
        );
    }
}

#[tokio::test]
async fn about_page_200_with_cache_headers() {
    let (status, headers, body) = req(app(), "GET", "/about", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=43200, s-maxage=43200, immutable"
    );
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("About"), "about should render its title");
    assert!(
        body.contains("Colophon"),
        "about should render markdown body"
    );
}

#[tokio::test]
async fn about_page_304_on_matching_etag() {
    let (_, headers, _) = req(app(), "GET", "/about", None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/about", Some(etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}

#[tokio::test]
async fn about_link_present_in_header() {
    let (_, _, body) = req(app(), "GET", "/", None).await;
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains(r#"<a href="/about">About</a>"#),
        "base layout should link to /about"
    );
}

#[tokio::test]
async fn teapot_returns_418_with_poem() {
    let (status, headers, body) = req(app(), "GET", "/teapot", None).await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("short and stout"));
    assert!(body.contains("pour me out"));
}

#[tokio::test]
async fn feed_returns_atom_xml_with_correct_content_type() {
    let (status, headers, body) = req(app(), "GET", "/feed.xml", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/atom+xml; charset=utf-8"
    );
    assert!(headers.contains_key(header::ETAG));
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=43200, s-maxage=43200, immutable"
    );
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("<feed"), "should have feed root element");
    assert!(body.contains("<entry>"), "should contain entries");
    assert!(
        body.contains("<title>Hello World</title>"),
        "should contain post title"
    );
    assert!(
        body.contains("<title>Second Post</title>"),
        "should contain second post"
    );
    assert!(
        body.contains(r#"type="html""#),
        "full-content feed should use html content type"
    );
    assert!(
        body.contains("about cambodian vodka"),
        "full-content feed should include post body HTML"
    );
    assert!(
        body.contains("<category"),
        "feed entries should include tag categories"
    );
}

#[tokio::test]
async fn feed_redirect_to_feed_xml() {
    let (status, headers, body) = req(app(), "GET", "/feed", None).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/feed.xml");
    assert!(body.is_empty(), "redirect body should be empty");
}

#[tokio::test]
async fn feed_304_on_matching_etag() {
    let (_, headers, _) = req(app(), "GET", "/feed.xml", None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/feed.xml", Some(etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}
