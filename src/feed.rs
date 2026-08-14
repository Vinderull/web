use crate::SITE_AUTHOR;
use crate::SITE_NAME;
use crate::SITE_URL;
use crate::posts::Post;

/// Minimal XML escape for text and attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convert a `YYYY-MM-DD` date string to an RFC 3339 timestamp at midnight UTC.
fn date_to_rfc3339(date_str: &str) -> String {
    let mut s = String::with_capacity(25);
    s.push_str(date_str);
    s.push_str("T00:00:00+00:00");
    s
}

/// Build an Atom feed XML string from all posts. Pre-rendered once at boot.
pub fn build_feed(posts: &[Post]) -> String {
    let mut xml = String::with_capacity(posts.len() * 512 + 512);

    xml.push_str("<?xml version=\"1.0\"?>\n");
    xml.push_str(r#"<feed xmlns="http://www.w3.org/2005/Atom">"#);

    // Title
    xml.push_str("<title>");
    xml.push_str(&esc(SITE_NAME));
    xml.push_str("</title>");

    // ID (canonical IRI: trailing slash per RFC 4287 §4.2.6)
    xml.push_str("<id>");
    xml.push_str(SITE_URL);
    xml.push_str("/</id>");

    // updated = newest post's date (posts are sorted desc)
    if let Some(newest) = posts.first() {
        xml.push_str("<updated>");
        xml.push_str(&date_to_rfc3339(&newest.date));
        xml.push_str("</updated>");
    }

    // Self link
    xml.push_str(r#"<link href=""#);
    xml.push_str(&esc(&format!("{SITE_URL}/feed.xml")));
    xml.push_str(r#"" rel="self"/>"#);

    // Alternate link
    xml.push_str(r#"<link href=""#);
    xml.push_str(&esc(SITE_URL));
    xml.push_str(r#"" rel="alternate"/>"#);

    // Author (feed-level; covers all entries per RFC 4287)
    xml.push_str("<author><name>");
    xml.push_str(&esc(SITE_AUTHOR));
    xml.push_str("</name></author>");

    // Entries
    for post in posts {
        let rfc3339 = date_to_rfc3339(&post.date);

        xml.push_str("<entry>");

        // Title
        xml.push_str("<title>");
        xml.push_str(&esc(&post.title));
        xml.push_str("</title>");

        // ID
        xml.push_str("<id>");
        xml.push_str(&esc(&format!("{SITE_URL}/posts/{}", post.slug)));
        xml.push_str("</id>");

        // updated
        xml.push_str("<updated>");
        xml.push_str(&rfc3339);
        xml.push_str("</updated>");

        // Tags as categories (before link in atom_syndication output)
        for tag in &post.tags {
            xml.push_str(r#"<category term=""#);
            xml.push_str(&esc(tag));
            xml.push_str(r#""/>"#);
        }

        // Alternate link
        xml.push_str(r#"<link href=""#);
        xml.push_str(&esc(&format!("{SITE_URL}/posts/{}", post.slug)));
        xml.push_str(r#"" rel="alternate"/>"#);

        // published
        xml.push_str("<published>");
        xml.push_str(&rfc3339);
        xml.push_str("</published>");

        // Summary (plain text, before content in atom_syndication output)
        if let Some(ref desc) = post.description {
            xml.push_str("<summary>");
            xml.push_str(&esc(desc));
            xml.push_str("</summary>");
        }

        // Content (HTML, entity-escaped)
        xml.push_str(r#"<content type="html">"#);
        xml.push_str(&esc(&post.html));
        xml.push_str("</content>");

        xml.push_str("</entry>");
    }

    xml.push_str("</feed>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_post(slug: &str, title: &str, date: &str) -> Post {
        Post {
            slug: slug.to_string(),
            title: title.to_string(),
            date: date.to_string(),
            date_display: String::new(),
            description: None,
            tags: Vec::new(),
            html: format!("<p>Body of {}</p>", slug),
            toc: String::new(),
            reading_time: 1,
        }
    }

    #[test]
    fn feed_contains_root_and_entries() {
        let posts = vec![
            fake_post("second", "Second Post", "2025-06-15"),
            fake_post("first", "First Post", "2025-01-01"),
        ];
        let xml = build_feed(&posts);

        assert!(xml.starts_with("<?xml version=\"1.0\"?>"));
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(xml.contains("<entry>"));
        assert!(xml.contains("<title>Second Post</title>"));
        assert!(xml.contains("<title>First Post</title>"));
    }

    #[test]
    fn feed_xml_escapes_special_chars() {
        let mut post = fake_post("test", "A & B < C > D \"quoted\"", "2025-01-01");
        post.tags = vec!["a&b".into()];
        let xml = build_feed(&[post]);

        assert!(xml.contains("A &amp; B &lt; C &gt; D &quot;quoted&quot;"));
        assert!(xml.contains("term=\"a&amp;b\""));
    }

    #[test]
    fn feed_html_content_is_entity_escaped() {
        let mut post = fake_post("test", "Test", "2025-01-01");
        post.html = "<p>about cambodian vodka & htmx</p>".into();
        let xml = build_feed(&[post]);

        assert!(xml.contains("&lt;p&gt;about cambodian vodka &amp; htmx&lt;/p&gt;"));
    }

    #[test]
    fn feed_includes_updated_from_newest_post() {
        let posts = vec![
            fake_post("newest", "Newest", "2025-12-31"),
            fake_post("oldest", "Oldest", "2025-01-01"),
        ];
        let xml = build_feed(&posts);
        assert!(xml.contains("<updated>2025-12-31T00:00:00+00:00</updated>"));
    }

    #[test]
    fn feed_omits_updated_when_no_posts() {
        let xml = build_feed(&[]);
        assert!(!xml.contains("<updated>"));
        assert!(xml.contains("</feed>"));
    }

    #[test]
    fn feed_includes_summary_when_description_present() {
        let mut post = fake_post("test", "Test", "2025-01-01");
        post.description = Some("A first post".into());
        let xml = build_feed(&[post]);

        assert!(xml.contains("<summary>A first post</summary>"));
    }

    #[test]
    fn feed_omits_summary_when_description_absent() {
        let post = fake_post("test", "Test", "2025-01-01");
        let xml = build_feed(&[post]);

        assert!(!xml.contains("<summary>"));
    }

    #[test]
    fn feed_includes_tags_as_categories() {
        let mut post = fake_post("test", "Test", "2025-01-01");
        post.tags = vec!["rust".into(), "web".into()];
        let xml = build_feed(&[post]);

        assert!(xml.contains(r#"<category term="rust"/>"#));
        assert!(xml.contains(r#"<category term="web"/>"#));
    }

    #[test]
    fn feed_omits_categories_when_no_tags() {
        let post = fake_post("test", "Test", "2025-01-01");
        let xml = build_feed(&[post]);

        assert!(!xml.contains("<category"));
    }

    #[test]
    fn feed_includes_self_and_alternate_links() {
        let post = fake_post("test", "Test", "2025-01-01");
        let xml = build_feed(&[post]);

        assert!(xml.contains(r#"rel="self""#));
        assert!(xml.contains(r#"rel="alternate""#));
        // Entry has its own alternate link
        assert!(
            xml.matches(r#"rel="alternate""#).count() >= 2,
            "should have feed-level alternate + entry-level alternate"
        );
    }

    #[test]
    fn feed_published_matches_date() {
        let post = fake_post("test", "Test", "2025-06-15");
        let xml = build_feed(&[post]);

        assert!(xml.contains("<published>2025-06-15T00:00:00+00:00</published>"));
    }

    #[test]
    fn feed_handles_multiple_posts_with_all_fields() {
        let mut p1 = fake_post("first", "First", "2025-01-01");
        p1.description = Some("desc one".into());
        p1.tags = vec!["rust".into()];

        let mut p2 = fake_post("second", "Second", "2025-06-15");
        p2.description = Some("desc two".into());
        p2.tags = vec!["web".into(), "htmx".into()];

        let xml = build_feed(&[p2, p1]);

        // Order: newest first (p2 then p1)
        let p2_start = xml.find("<title>Second</title>").unwrap();
        let p1_start = xml.find("<title>First</title>").unwrap();
        assert!(p2_start < p1_start, "newest post should come first");

        assert!(xml.contains("<summary>desc one</summary>"));
        assert!(xml.contains("<summary>desc two</summary>"));
    }

    // ── RFC 4287 compliance ──

    #[test]
    fn rfc4287_feed_has_namespace() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        assert!(xml.contains(r#"<feed xmlns="http://www.w3.org/2005/Atom">"#));
    }

    #[test]
    fn rfc4287_feed_has_exactly_one_id() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        // Feed-level <id>, not entry-level. Count openings.
        let feed_part = &xml[..xml.find("<entry>").unwrap()];
        assert_eq!(
            feed_part.matches("<id>").count(),
            1,
            "feed must have exactly one atom:id"
        );
    }

    #[test]
    fn rfc4287_feed_has_exactly_one_title() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        let feed_part = &xml[..xml.find("<entry>").unwrap()];
        assert_eq!(
            feed_part.matches("<title>").count(),
            1,
            "feed must have exactly one atom:title"
        );
    }

    #[test]
    fn rfc4287_feed_has_exactly_one_updated() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        let feed_part = &xml[..xml.find("<entry>").unwrap()];
        assert_eq!(
            feed_part.matches("<updated>").count(),
            1,
            "feed must have exactly one atom:updated"
        );
    }

    #[test]
    fn rfc4287_entry_has_exactly_one_id_title_updated() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        // Isolate first entry
        let entry_start = xml.find("<entry>").unwrap();
        let entry_end = xml[entry_start..].find("</entry>").unwrap();
        let entry = &xml[entry_start..entry_start + entry_end];

        assert_eq!(
            entry.matches("<id>").count(),
            1,
            "entry must have exactly one atom:id"
        );
        assert_eq!(
            entry.matches("<title>").count(),
            1,
            "entry must have exactly one atom:title"
        );
        assert_eq!(
            entry.matches("<updated>").count(),
            1,
            "entry must have exactly one atom:updated"
        );
    }

    #[test]
    fn rfc4287_updated_is_rfc3339() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-15")]);
        // The updated element content must match RFC 3339 date-time
        assert!(xml.contains("<updated>2025-01-15T00:00:00+00:00</updated>"));
    }

    #[test]
    fn rfc4287_content_has_type_attribute() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        assert!(xml.contains(r#"<content type="html">"#));
    }

    #[test]
    fn rfc4287_id_is_valid_iri() {
        // atom:id MUST be a valid IRI (RFC 3987). Our IDs are absolute HTTPS URLs.
        let xml = build_feed(&[fake_post("my-post", "T", "2025-01-01")]);
        assert!(xml.contains(&format!("<id>{}/</id>", SITE_URL)));
        assert!(xml.contains(&format!("<id>{}/posts/my-post</id>", SITE_URL)));
    }

    #[test]
    fn rfc4287_self_link_present() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        // The feed should have a self link (RFC 4287 recommends this)
        assert!(xml.contains(r#"rel="self""#));
    }

    #[test]
    fn rfc4287_feed_has_author() {
        let xml = build_feed(&[fake_post("t", "T", "2025-01-01")]);
        // RFC 4287: atom:feed MUST contain at least one atom:author
        assert!(xml.contains("<author><name>"));
        assert!(xml.contains("</name></author>"));
        // Author name is the SITE_AUTHOR constant
        assert!(xml.contains(&format!("<name>{}</name>", SITE_AUTHOR)));
    }

    #[test]
    fn rfc4287_all_five_xml_special_chars_escaped() {
        let mut post = fake_post("t", "The \"5\" & <escape>", "2025-01-01");
        post.html = r#"<p>if a < b && c > 'd'</p>"#.into();
        post.description = Some(r#"amp & lt < gt >"#.into());
        let xml = build_feed(&[post]);

        // Title
        assert!(xml.contains("The &quot;5&quot; &amp; &lt;escape&gt;"));
        // Content: < > & escaped; " and ' too
        assert!(xml.contains("&lt;p&gt;if a &lt; b &amp;&amp; c &gt; &apos;d&apos;&lt;/p&gt;"));
        // Summary
        assert!(xml.contains("amp &amp; lt &lt; gt &gt;"));
    }
}
