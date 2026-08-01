use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use serde::Deserialize;
use time::macros::format_description;

#[derive(Debug, Clone)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub date: String,
    pub date_display: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub html: String,
    pub toc: String,
}

#[derive(Deserialize)]
struct FrontMatter {
    title: String,
    date: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

pub fn load_all(content_dir: &Path) -> Result<Vec<Post>> {
    let posts_dir = content_dir.join("posts");
    let mut posts = Vec::new();

    if !posts_dir.exists() {
        tracing::warn!("Posts directory not found: {}", posts_dir.display());
        return Ok(posts);
    }

    for entry in std::fs::read_dir(&posts_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid filename")?
            .to_string();

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;

        let post =
            parse_post(&slug, &content).with_context(|| format!("parsing {}", path.display()))?;
        posts.push(post);
    }

    posts.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(posts)
}

fn parse_post(slug: &str, content: &str) -> Result<Post> {
    let (frontmatter, markdown) = split_frontmatter(content);
    let fm: FrontMatter = toml::from_str(&frontmatter).context("parsing frontmatter")?;

    let (html, toc) = render_markdown(markdown);
    let date_display = format_date(&fm.date)?;

    Ok(Post {
        slug: slug.to_string(),
        title: fm.title,
        date: fm.date,
        date_display,
        description: fm.description,
        tags: fm.tags,
        html,
        toc,
    })
}

/// Split TOML frontmatter (delimited by `+++`) from markdown body.
fn split_frontmatter(content: &str) -> (String, &str) {
    let content = content.trim_start();
    let marker = "+++";
    if !content.starts_with(marker) {
        return (String::new(), content);
    }

    let after_open = &content[marker.len()..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    match after_open.find(marker) {
        Some(end) => {
            let frontmatter = after_open[..end].to_string();
            let markdown = after_open[end + marker.len()..].trim_start_matches(['\r', '\n']);
            (frontmatter, markdown)
        }
        None => (String::new(), content),
    }
}

// heading levels collected for the ToC when building the body in one pass
struct TocNode {
    level: u32,
    id: String,
    text: String,
    children: Vec<usize>,
}

/// Render markdown to sanitized HTML plus a table of contents (empty string
/// when there are no h2+ headings). Both share one parse pass: heading text is
/// collected, slugged, and written back as `id` attributes on the emitted
/// headings so ToC anchors resolve within the body.
fn render_markdown(markdown: &str) -> (String, String) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let events: Vec<Event<'_>> = Parser::new_ext(markdown, options).collect();

    let mut used = HashSet::new();
    let mut headings: Vec<(u32, String, String)> = Vec::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let (level, text, end) = match &events[i] {
            Event::Start(Tag::Heading { level, .. }) => {
                // Gather everything up to this heading's matching End.
                let mut nested = 0usize;
                let mut inner = Vec::new();
                let mut j = i + 1;
                loop {
                    match events[j] {
                        Event::Start(Tag::Heading { .. }) => nested += 1,
                        Event::End(TagEnd::Heading(_)) if nested == 0 => break,
                        Event::End(TagEnd::Heading(_)) => nested -= 1,
                        _ => {}
                    }
                    inner.push(events[j].clone());
                    j += 1;
                }
                let mut text = String::new();
                for e in &inner {
                    if let Event::Text(t) | Event::Code(t) = e {
                        text.push_str(t);
                    }
                }
                (*level as u32, text, j)
            }
            _ => {
                out.push(events[i].clone());
                i += 1;
                continue;
            }
        };

        let id = unique_slug(&text, &mut used);
        let mut start = events[i].clone();
        if let Event::Start(Tag::Heading { id: slot, .. }) = &mut start {
            *slot = Some(id.clone().into());
        }
        out.push(start);
        out.extend(events[i + 1..end].iter().cloned());
        out.push(events[end].clone());
        headings.push((level, id, text));
        i = end + 1;
    }

    let mut raw_html = String::new();
    html::push_html(&mut raw_html, out.into_iter());

    let mut cleaner = ammonia::Builder::new();
    cleaner
        .add_tags(&["input"])
        .add_generic_attributes(&["id"])
        .add_tag_attributes("input", &["type", "checked", "disabled"]);
    let body = cleaner.clean(&raw_html).to_string();

    (body, build_toc(&headings))
}

/// Turn heading text into a URL-safe, lowercase anchor id.
fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                slug.push(l);
            }
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Slug with a numeric suffix on collision so anchors stay unique.
fn unique_slug(text: &str, used: &mut HashSet<String>) -> String {
    let base = {
        let s = slugify(text);
        if s.is_empty() {
            "section".to_string()
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// Render collected (level, id, text) headings into a nested `<ul>` ToC.
/// Includes h2+ only (h1 is conventionally the post title). Levels may skip
/// (h2 then h4): such gaps are flattened by parent assignment.
fn build_toc(headings: &[(u32, String, String)]) -> String {
    let root = TocNode {
        level: 0,
        id: String::new(),
        text: String::new(),
        children: Vec::new(),
    };
    let mut arena: Vec<TocNode> = vec![root];
    let mut stack: Vec<usize> = vec![0];

    for (level, id, text) in headings {
        if *level < 2 {
            continue;
        }
        while arena[*stack.last().unwrap()].level >= *level {
            stack.pop();
        }
        let idx = arena.len();
        arena.push(TocNode {
            level: *level,
            id: id.clone(),
            text: text.clone(),
            children: Vec::new(),
        });
        arena[*stack.last().unwrap()].children.push(idx);
        stack.push(idx);
    }

    let mut out = String::new();
    render_toc(&arena, 0, &mut out);
    out
}

fn render_toc(arena: &[TocNode], idx: usize, out: &mut String) {
    let children = &arena[idx].children;
    if children.is_empty() {
        return;
    }
    out.push_str("<ul>");
    for &c in children {
        let child = &arena[c];
        out.push_str(&format!("<li><a href=\"#{}\">{}</a>", child.id, child.text));
        render_toc(arena, c, out);
        out.push_str("</li>");
    }
    out.push_str("</ul>");
}

fn format_date(date_str: &str) -> Result<String> {
    let date = time::Date::parse(date_str, format_description!("[year]-[month]-[day]"))
        .context("parsing date (expected YYYY-MM-DD)")?;
    Ok(date.format(format_description!("[month repr:long] [day], [year]"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_with_metadata() {
        let input = "+++\ntitle = \"Test\"\ndate = \"2024-01-15\"\n+++\n# Hello";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.contains("title = \"Test\""));
        assert_eq!(md, "# Hello");
    }

    #[test]
    fn test_split_frontmatter_without_metadata() {
        let input = "# Just markdown";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.is_empty());
        assert_eq!(md, "# Just markdown");
    }

    #[test]
    fn test_parse_post() {
        let input = "+++\ntitle = \"Test Post\"\ndate = \"2024-01-15\"\ndescription = \"A test\"\n+++\n\nHello **world**";
        let post = parse_post("test-post", input).unwrap();
        assert_eq!(post.slug, "test-post");
        assert_eq!(post.title, "Test Post");
        assert_eq!(post.date, "2024-01-15");
        assert_eq!(post.description, Some("A test".to_string()));
        assert_eq!(post.tags, Vec::<String>::new(), "no tags means empty list");
        assert!(post.html.contains("<strong>world</strong>"));
        assert!(post.date_display.contains("January"));
    }

    #[test]
    fn test_render_markdown() {
        let (html, _) = render_markdown("# Title\n\n- item 1\n- item 2");
        assert!(html.contains("<h1 id=\"title\">Title</h1>"));
        assert!(html.contains("<li>item 1</li>"));
    }

    #[test]
    fn test_split_frontmatter_unclosed() {
        let input = "+++\ntitle = \"Test\"\n# no closing marker";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.is_empty(), "unclosed frontmatter yields empty fm");
        assert!(md.contains("no closing marker"));
    }

    #[test]
    fn test_split_frontmatter_empty_block() {
        let input = "+++\n+++\nBody here";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.is_empty());
        assert_eq!(md, "Body here");
    }

    #[test]
    fn test_split_frontmatter_leading_whitespace() {
        let input = "\n\n  +++\ntitle = \"T\"\n+++\nBody";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.contains("title = \"T\""));
        assert_eq!(md, "Body");
    }

    #[test]
    fn test_parse_post_missing_title() {
        let input = "+++\ndate = \"2024-01-15\"\n+++\nbody";
        assert!(parse_post("s", input).is_err());
    }

    #[test]
    fn test_parse_post_missing_date() {
        let input = "+++\ntitle = \"T\"\n+++\nbody";
        assert!(parse_post("s", input).is_err());
    }

    #[test]
    fn test_parse_post_tags() {
        let input =
            "+++\ntitle = \"T\"\ndate = \"2024-01-15\"\ntags = [\"rust\", \"web\"]\n+++\nbody";
        let post = parse_post("s", input).unwrap();
        assert_eq!(post.tags, vec!["rust", "web"]);
    }

    #[test]
    fn test_parse_post_no_description_defaults_none() {
        let input = "+++\ntitle = \"T\"\ndate = \"2024-01-15\"\n+++\nbody";
        let post = parse_post("s", input).unwrap();
        assert_eq!(post.description, None);
    }

    #[test]
    fn test_parse_post_empty_body() {
        let input = "+++\ntitle = \"T\"\ndate = \"2024-01-15\"\n+++\n";
        let post = parse_post("s", input).unwrap();
        assert!(post.html.trim().is_empty());
    }

    #[test]
    fn test_format_date_valid() {
        assert_eq!(format_date("2024-01-15").unwrap(), "January 15, 2024");
        assert_eq!(format_date("2023-12-31").unwrap(), "December 31, 2023");
    }

    #[test]
    fn test_format_date_invalid_format() {
        assert!(format_date("01/15/2024").is_err());
        assert!(format_date("not-a-date").is_err());
        assert!(format_date("").is_err());
    }

    #[test]
    fn test_render_markdown_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let (html, _) = render_markdown(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_render_markdown_strikethrough() {
        let (html, _) = render_markdown("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_render_markdown_code_block() {
        let (html, _) = render_markdown("```\nlet x = 1;\n```");
        assert!(html.contains("<code>"));
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn test_render_markdown_tasklist() {
        let (html, _) = render_markdown("- [x] done\n- [ ] todo");
        assert!(
            html.contains("task-list") || html.contains("checkbox") || html.contains("disabled")
        );
    }

    #[test]
    fn test_render_markdown_toc_nested() {
        let md = "# Title\n\n## Section A\n\n### Sub one\n\n### Sub two\n\n## Section B";
        let (html, toc) = render_markdown(md);
        // h1 (the title) is excluded from the ToC but still gets an id.
        assert!(html.contains("<h1 id=\"title\">Title</h1>"));
        assert!(html.contains("<h2 id=\"section-a\">Section A</h2>"));
        // Nested list: Section A contains its two subsections.
        assert!(
            toc.contains("<ul><li><a href=\"#section-a\">Section A</a><ul>")
                && toc.contains("<li><a href=\"#sub-one\">Sub one</a>")
                && toc.contains("<li><a href=\"#sub-two\">Sub two</a>")
        );
        assert!(
            toc.contains("</li><li><a href=\"#section-b\">Section B</a></li></ul>"),
            "Section B closes the top-level list: {toc}"
        );
        // h1 title must not appear in the ToC.
        assert!(!toc.contains("title"));
    }

    #[test]
    fn test_render_markdown_toc_empty_when_no_headings() {
        let (_, toc) = render_markdown("just **text**, no headings");
        assert!(toc.is_empty());
    }

    #[test]
    fn test_render_markdown_toc_only_h1() {
        let (html, toc) = render_markdown("# Just a title");
        assert!(html.contains("<h1 id=\"just-a-title\">"));
        assert!(toc.is_empty(), "h1-only posts get no ToC");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Rust & C++ 101"), "rust-c-101");
        assert_eq!(slugify("  ---  "), "");
    }

    #[test]
    fn test_unique_slug_dedupes() {
        let mut used = HashSet::new();
        assert_eq!(unique_slug("Conclusion", &mut used), "conclusion");
        assert_eq!(unique_slug("Conclusion", &mut used), "conclusion-2");
        assert_eq!(unique_slug("Conclusion", &mut used), "conclusion-3");
    }

    #[test]
    fn test_load_all_missing_dir() {
        let dir = std::env::temp_dir().join("web_test_nonexistent_posts_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let posts = load_all(&dir).unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn test_load_all_empty_dir() {
        let dir = std::env::temp_dir().join("web_test_empty_posts");
        std::fs::create_dir_all(dir.join("posts")).unwrap();
        let posts = load_all(&dir).unwrap();
        assert!(posts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_all_sorts_by_date_desc() {
        let dir = std::env::temp_dir().join("web_test_sort_posts");
        let posts_subdir = dir.join("posts");
        std::fs::create_dir_all(&posts_subdir).unwrap();

        std::fs::write(
            posts_subdir.join("older.md"),
            "+++\ntitle = \"Old\"\ndate = \"2023-01-01\"\n+++\nold",
        )
        .unwrap();
        std::fs::write(
            posts_subdir.join("newer.md"),
            "+++\ntitle = \"New\"\ndate = \"2024-12-31\"\n+++\nnew",
        )
        .unwrap();
        // Non-markdown file should be ignored
        std::fs::write(posts_subdir.join("notes.txt"), "ignore me").unwrap();

        let posts = load_all(&dir).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].slug, "newer", "newest post first");
        assert_eq!(posts[1].slug, "older");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
