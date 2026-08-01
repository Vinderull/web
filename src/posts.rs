use std::path::Path;

use anyhow::{Context, Result};
use pulldown_cmark::{Options, Parser, html};
use serde::Deserialize;
use time::macros::format_description;

#[derive(Debug, Clone)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub date: String,
    pub date_display: String,
    pub description: Option<String>,
    pub html: String,
}

#[derive(Deserialize)]
struct FrontMatter {
    title: String,
    date: String,
    #[serde(default)]
    description: Option<String>,
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

    let html = render_markdown(markdown);
    let date_display = format_date(&fm.date)?;

    Ok(Post {
        slug: slug.to_string(),
        title: fm.title,
        date: fm.date,
        date_display,
        description: fm.description,
        html,
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

fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, options);
    let mut raw_html = String::new();
    html::push_html(&mut raw_html, parser);

    let mut cleaner = ammonia::Builder::new();
    cleaner
        .add_tags(&["input"])
        .add_tag_attributes("input", &["type", "checked", "disabled"]);
    cleaner.clean(&raw_html).to_string()
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
        assert!(post.html.contains("<strong>world</strong>"));
        assert!(post.date_display.contains("January"));
    }

    #[test]
    fn test_render_markdown() {
        let html = render_markdown("# Title\n\n- item 1\n- item 2");
        assert!(html.contains("<h1>Title</h1>"));
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
        let html = render_markdown(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_render_markdown_strikethrough() {
        let html = render_markdown("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_render_markdown_code_block() {
        let html = render_markdown("```\nlet x = 1;\n```");
        assert!(html.contains("<code>"));
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn test_render_markdown_tasklist() {
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(
            html.contains("task-list") || html.contains("checkbox") || html.contains("disabled")
        );
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
