use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;
use web::{PERMISSIONS_POLICY_VALUE, build_app, posts};

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
///
/// `extra` lets a caller send one additional header — e.g. htmx's
/// `HX-Request: true` to request the bare search fragment.
async fn req(
    app: Router,
    method: &str,
    uri: &str,
    inm: Option<&str>,
    extra: Option<(&str, &str)>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(e) = inm {
        builder = builder.header(header::IF_NONE_MATCH, e);
    }
    if let Some((name, value)) = extra {
        builder = builder.header(name, value);
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
    let (status, headers, body) = req(app(), "GET", "/", None, None).await;
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
    let (_, headers, _) = req(app(), "GET", "/", None, None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/", Some(etag), None).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}

#[tokio::test]
async fn post_page_200_for_known_slug() {
    // The "second-post" fixture carries a precomputed ToC, so this request
    // exercises the ToC nav rather than a bare article.
    let slug = "second-post";
    let (status, headers, body) = req(app(), "GET", &format!("/posts/{slug}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key(header::ETAG));
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("<article>"),
        "post should render article wrapper"
    );
    assert!(
        body.contains("Table of Contents") && body.contains("class=\"toc\""),
        "post page should include the ToC nav"
    );
}

#[tokio::test]
async fn post_404_for_unknown_slug() {
    let (status, headers, _) = req(app(), "GET", "/posts/does-not-exist", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store",
        "custom 404 pages must never be cached"
    );
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (status, _, body) = req(app(), "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
}

#[tokio::test]
async fn static_asset_served() {
    let (status, _, body) = req(app(), "GET", "/static/css/main.css", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.is_empty(), "css should have content");
}

#[tokio::test]
async fn favicon_is_advertised_and_served() {
    let (status, _, body) = req(app(), "GET", "/", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains(r#"<link rel="icon" type="image/svg+xml" href="/static/favicon.svg">"#),
        "full pages should advertise the SVG favicon"
    );

    let (status, headers, body) = req(app(), "GET", "/static/favicon.svg", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/svg+xml");
    assert!(!body.is_empty(), "favicon should have content");
}

#[tokio::test]
async fn static_htmx_matches_sri_integrity() {
    // The bytes served, the hash pinned by scripts/update-htmx.sh, and the
    // integrity attribute on the <script> tag in templates/base.html must be
    // one and the same, or htmx is broken for every visitor with a CSP/SRI
    // checking browser. Recompute the constants with the same source the
    // vendor script uses (unpkg's published integrity field):
    //   SHA256 (sha256sum) = unpkg /htmx.org@<version>/?meta -> dist/htmx.min.js
    //   SHA384 (SRI)       = openssl dgst -sha384 -binary static/js/htmx.min.js \
    //                        | openssl base64 -A
    // The vendor script prints the SRI value on every run.
    let (status, _, body) = req(app(), "GET", "/static/js/htmx.min.js", None, None).await;
    assert_eq!(status, StatusCode::OK);

    let sha256: [u8; 32] = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&body).into()
    };
    let sha384: [u8; 48] = {
        use sha2::{Digest, Sha384};
        Sha384::digest(&body).into()
    };

    let expected_sha256 = "e484d9171a9db30a39c8f16e3d709d4137f3211c659f8e6125816635033d593f";
    assert_eq!(
        hex(&sha256),
        expected_sha256,
        "served htmx.min.js must be the pinned release from scripts/update-htmx.sh"
    );

    let expected_integrity = "BvJpBiO8Kh31EqtJe5DRIeWrHWnCGkwytKs9NKFi86Hhw96dEqdEMzZDeK9iEGTc";
    assert_eq!(
        base64_sha384(&sha384),
        expected_integrity,
        "SRI integrity in templates/base.html must match the served htmx.min.js"
    );

    // The <script> tag in the layout must carry the value SRI-enforcing
    // browsers validate the served bytes against.
    let base = std::fs::read_to_string("templates/base.html")
        .expect("templates/base.html must exist next to the tests");
    assert!(
        base.contains(&format!("integrity=\"sha384-{expected_integrity}\"")),
        "templates/base.html must pin the served htmx.min.js integrity value"
    );
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn base64_sha384(bytes: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[tokio::test]
async fn static_asset_missing_returns_404() {
    let (status, _, _) = req(app(), "GET", "/static/nope.css", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn index_sets_security_headers() {
    let (_, headers, _) = req(app(), "GET", "/", None, None).await;
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
        headers.get("permissions-policy").unwrap(),
        PERMISSIONS_POLICY_VALUE,
    );
}

#[tokio::test]
async fn index_carries_exact_strict_csp() {
    // Pin the literal header: the other CSP assertions compare against
    // `CSP_VALUE`, so a regression in the constant itself is only caught by
    // asserting the exact policy string on a rendered response.
    let (_, headers, _) = req(app(), "GET", "/", None, None).await;
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        "default-src 'none'; base-uri 'none'; connect-src 'self'; form-action 'none'; frame-ancestors 'none'; img-src 'self'; script-src 'self'; style-src 'self'",
    );
}

#[tokio::test]
async fn post_page_304_on_matching_etag() {
    let slug = "hello-world";
    let app = app();
    let uri = format!("/posts/{slug}");
    let (status, headers, _) = req(app.clone(), "GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app, "GET", &uri, Some(etag), None).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}

// "cambodian" appears only in the "hello-world" fixture's body, isolating it.
const QUERY: &str = "/search?q=cambodian";

#[tokio::test]
async fn search_returns_fragment_for_htmx() {
    let (status, headers, body) =
        req(app(), "GET", QUERY, None, Some(("HX-Request", "true"))).await;
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
    let (status, _, body) = req(app(), "GET", QUERY, None, None).await;
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
    let (_, headers, _) = req(app(), "GET", "/search?q=hello", None, None).await;
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
    assert!(
        !posts.is_empty(),
        "fixed test corpus must provide posts for the empty-query search"
    );
    let (status, _, body) = req(app(), "GET", "/search?q=", None, None).await;
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
    let (status, _, body) = req(app(), "GET", "/search?q=zzzznomatch", None, None).await;
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
    let (status, _, body) = req(app(), "GET", "/tags", None, None).await;
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
    let tag = posts
        .iter()
        .flat_map(|p| &p.tags)
        .next()
        .expect("fixed test corpus must include tagged posts")
        .clone();
    let (status, headers, body) = req(app(), "GET", &format!("/tags/{tag}"), None, None).await;
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
    let (status, headers, _) = req(app(), "GET", "/tags/does-not-exist", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store",
        "custom 404 pages must never be cached"
    );
}

#[tokio::test]
async fn post_page_shows_tags() {
    let posts = test_corpus();
    let post = posts
        .iter()
        .find(|p| !p.tags.is_empty())
        .expect("fixed test corpus must include a tagged post");
    let (status, _, body) = req(app(), "GET", &format!("/posts/{}", post.slug), None, None).await;
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
    let (status, headers, body) = req(app(), "GET", "/about", None, None).await;
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
    let (_, headers, _) = req(app(), "GET", "/about", None, None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/about", Some(etag), None).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}

#[tokio::test]
async fn about_link_present_in_header() {
    let (_, _, body) = req(app(), "GET", "/", None, None).await;
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains(r#"<a href="/about">About</a>"#),
        "base layout should link to /about"
    );
}

#[tokio::test]
async fn teapot_returns_418_with_poem() {
    let (status, headers, body) = req(app(), "GET", "/teapot", None, None).await;
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
    let (status, headers, body) = req(app(), "GET", "/feed.xml", None, None).await;
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
    let (status, headers, body) = req(app(), "GET", "/feed", None, None).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/feed.xml");
    assert!(body.is_empty(), "redirect body should be empty");
}

#[tokio::test]
async fn feed_304_on_matching_etag() {
    let (_, headers, _) = req(app(), "GET", "/feed.xml", None, None).await;
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, body) = req(app(), "GET", "/feed.xml", Some(etag), None).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "304 must have no body");
}
