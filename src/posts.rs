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
    pub reading_time: u32,
}

/// A standalone page (e.g. `/about`) — not a dated, listable post. Rendered
/// to pre-baked HTML at boot like posts, but with no date/tags/ToC/reading
/// time. Loaded from `content/pages/`.
#[derive(Debug, Clone)]
pub struct Page {
    pub slug: String,
    pub title: String,
    pub html: String,
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

#[derive(Deserialize)]
struct PageFrontMatter {
    title: String,
}

pub fn load_all(content_dir: &Path) -> Result<Vec<Post>> {
    let posts_dir = content_dir.join("posts");
    let mut posts = Vec::new();

    if !posts_dir.exists() {
        eprintln!(
            "Warning: posts directory not found: {}",
            posts_dir.display()
        );
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

/// Load standalone pages from `content/pages/*.md`. Same markdown pipeline as
/// posts, but frontmatter only carries a `title` (no date/tags). Returns an
/// empty vec if `content/pages` doesn't exist.
pub fn load_pages(content_dir: &Path) -> Result<Vec<Page>> {
    let pages_dir = content_dir.join("pages");
    let mut pages = Vec::new();

    if !pages_dir.exists() {
        eprintln!(
            "Warning: pages directory not found: {}",
            pages_dir.display()
        );
        return Ok(pages);
    }

    for entry in std::fs::read_dir(&pages_dir)? {
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

        let page =
            parse_page(&slug, &content).with_context(|| format!("parsing {}", path.display()))?;
        pages.push(page);
    }

    Ok(pages)
}

fn parse_page(slug: &str, content: &str) -> Result<Page> {
    let (frontmatter, markdown) = split_frontmatter(content);
    let fm: PageFrontMatter = toml::from_str(&frontmatter).context("parsing frontmatter")?;
    let (html, _) = render_markdown(markdown).context("rendering markdown body")?;
    Ok(Page {
        slug: slug.to_string(),
        title: fm.title,
        html,
    })
}

fn parse_post(slug: &str, content: &str) -> Result<Post> {
    let (frontmatter, markdown) = split_frontmatter(content);
    let fm: FrontMatter = toml::from_str(&frontmatter).context("parsing frontmatter")?;

    let (html, toc) = render_markdown(markdown).context("rendering markdown body")?;
    let date_display = format_date(&fm.date)?;
    let reading_time = estimate_reading_time(markdown);

    Ok(Post {
        slug: slug.to_string(),
        title: fm.title,
        date: fm.date,
        date_display,
        description: fm.description,
        tags: fm.tags,
        html,
        toc,
        reading_time,
    })
}

/// Estimate reading time in whole minutes from the raw markdown word count
/// (≈200 words/min). Always at least 1 minute.
fn estimate_reading_time(markdown: &str) -> u32 {
    let words = markdown.split_whitespace().count();
    words.div_ceil(200).max(1) as u32
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

    let mut offset = 0;
    for line in after_open.split('\n') {
        if line.trim() == marker {
            let frontmatter = after_open[..offset].to_string();
            let markdown = after_open[offset + marker.len()..].trim_start_matches(['\r', '\n']);
            return (frontmatter, markdown);
        }
        offset += line.len() + 1;
    }

    (String::new(), content)
}

// heading levels collected for the ToC when building the body in one pass
struct TocNode {
    level: u32,
    id: String,
    text: String,
    children: Vec<usize>,
}

/// Render markdown to validated HTML plus a table of contents (empty string
/// when there are no h2+ headings). Both share one parse pass: heading text is
/// collected, slugged, and written back as `id` attributes on the emitted
/// headings so ToC anchors resolve within the body.
///
/// The validated event stream is rendered directly with `push_html`; no
/// post-hoc HTML sanitizer is applied. Authored raw HTML (block or inline) and
/// link/image destinations outside the allowed URL policy are rejected, with
/// error messages that distinguish the two so callers surface useful context.
fn render_markdown(markdown: &str) -> Result<(String, String)> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut parser = Parser::new_ext(markdown, options);
    let mut used = HashSet::new();
    let mut headings: Vec<(u32, String, String)> = Vec::new();
    let mut out = Vec::new();

    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                // Everything up to this heading's matching End is its content.
                let inner: Vec<_> = parser
                    .by_ref()
                    .take_while(|e| !matches!(e, Event::End(TagEnd::Heading(_))))
                    .collect();
                // The heading body is still an authored markdown event stream:
                // raw HTML and unsafe destinations inside a heading must be
                // rejected too.
                for e in &inner {
                    validate_event(e)?;
                }
                let mut text = String::new();
                for e in &inner {
                    if let Event::Text(t) | Event::Code(t) = e {
                        text.push_str(t);
                    }
                }
                let id = unique_slug(&text, &mut used);
                out.push(Event::Start(Tag::Heading {
                    level,
                    id: Some(id.clone().into()),
                    classes: Vec::new(),
                    attrs: Vec::new(),
                }));
                out.extend(inner);
                out.push(Event::End(TagEnd::Heading(level)));
                headings.push((level as u32, id, text));
            }
            other => {
                validate_event(&other)?;
                out.push(other);
            }
        }
    }

    let mut body = String::new();
    html::push_html(&mut body, out.into_iter());

    Ok((body, build_toc(&headings)))
}

/// Validate a single authored markdown event against the security policy.
///
/// Raw HTML (block or inline) is rejected outright, and every link/image
/// destination must satisfy `validate_destination`. Returns a message naming
/// the offending construct so errors carry useful context.
fn validate_event(ev: &Event<'_>) -> Result<()> {
    match ev {
        Event::Html(_) | Event::InlineHtml(_) => {
            Err(anyhow::anyhow!("raw HTML is not allowed in markdown"))
        }
        Event::Start(tag) => validate_start_tag(tag),
        Event::End(tag_end) => validate_end_tag(tag_end),
        // Leaf payloads carry no embedded structure of their own: text, code,
        // math, footnote references, line breaks, rules, and task markers are
        // all safe. Math events are admitted without this application enabling
        // `Options::ENABLE_MATH` (no math renderer is wired up; they are simply
        // not treated as a validation failure if they ever appear).
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::Rule
        | Event::TaskListMarker(_) => Ok(()),
    }
}

/// Allowlist of markdown start tags admitted from authored content. Every
/// pulldown-cmark 0.13 `Tag` variant is named explicitly so no upstream tag
/// can silently take the success path.
///
/// Link and image destinations are validated against `validate_destination`;
/// HTML blocks are rejected like their leaf `Event::Html` payloads; tags for
/// structures this application neither enables nor supports are rejected
/// explicitly. The match is exhaustive, so a newly added upstream `Tag`
/// variant fails to compile instead of silently validating.
fn validate_start_tag(tag: &Tag<'_>) -> Result<()> {
    match tag {
        Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
            validate_destination(dest_url).map_err(|detail| anyhow::anyhow!("{detail}"))
        }
        Tag::HtmlBlock => Err(anyhow::anyhow!("raw HTML is not allowed in markdown")),
        // Structures enabled by the parser options in `render_markdown`
        // (tables, footnotes, strikethrough, task lists) plus the core
        // CommonMark containers and inline spans.
        Tag::Paragraph
        | Tag::Heading { .. }
        | Tag::BlockQuote(_)
        | Tag::CodeBlock(_)
        | Tag::List(_)
        | Tag::Item
        | Tag::FootnoteDefinition(_)
        | Tag::Table(_)
        | Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::Emphasis
        | Tag::Strong
        | Tag::Strikethrough => Ok(()),
        // Structures this application does not enable or support (definition
        // lists, super/subscript, metadata blocks). Rejected explicitly so
        // they can never slip through validation.
        Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::Superscript
        | Tag::Subscript
        | Tag::MetadataBlock(_) => Err(anyhow::anyhow!("unsupported markdown structure")),
    }
}

/// Exhaustive counterpart to `validate_start_tag` for closing tags. Mirrors
/// the start-tag policy so a closing tag can never outrun the corresponding
/// rejected opening structure.
fn validate_end_tag(tag_end: &TagEnd) -> Result<()> {
    match tag_end {
        TagEnd::HtmlBlock => Err(anyhow::anyhow!("raw HTML is not allowed in markdown")),
        TagEnd::DefinitionList
        | TagEnd::DefinitionListTitle
        | TagEnd::DefinitionListDefinition
        | TagEnd::Superscript
        | TagEnd::Subscript
        | TagEnd::MetadataBlock(_) => Err(anyhow::anyhow!("unsupported markdown structure")),
        TagEnd::Paragraph
        | TagEnd::Heading(_)
        | TagEnd::BlockQuote(_)
        | TagEnd::CodeBlock
        | TagEnd::List(_)
        | TagEnd::Item
        | TagEnd::FootnoteDefinition
        | TagEnd::Table
        | TagEnd::TableHead
        | TagEnd::TableRow
        | TagEnd::TableCell
        | TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Link
        | TagEnd::Image => Ok(()),
    }
}

/// Allowed link/image destination policy: ordinary relative URLs and
/// fragments, plus case-insensitive `http`, `https`, and `mailto` schemes.
/// Protocol-relative URLs, backslashes, ASCII whitespace/control characters,
/// and every other explicit scheme (e.g. `javascript`, `data`, `file`, `ftp`)
/// are rejected. Nothing is allocated on the success path.
fn validate_destination(dest: &str) -> std::result::Result<(), String> {
    let err = |detail: String| format!("unsafe link destination {dest:?}: {detail}");
    if dest.starts_with("//") {
        return Err(err("protocol-relative URLs are not allowed".to_string()));
    }
    if dest.contains('\\') {
        return Err(err("backslashes are not allowed".to_string()));
    }
    let bytes = dest.as_bytes();
    if bytes.iter().any(|&b| b < 0x21 || b == 0x7f) {
        return Err(err("contains whitespace or control characters".to_string()));
    }
    if let Some(colon) = scheme_end(dest) {
        let scheme = &dest[..colon];
        if !matches_scheme(scheme) {
            return Err(err(format!("scheme '{scheme}' is not allowed")));
        }
    }
    Ok(())
}

/// Index of the `:` ending a URL scheme prefix, or `None` when the leading
/// characters do not form an RFC 3986 scheme (`ALPHA *( ALPHA / DIGIT / "+" /
/// "-" / "." )`).
fn scheme_end(dest: &str) -> Option<usize> {
    let mut chars = dest.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return None,
    }
    for (i, c) in chars {
        match c {
            ':' => return Some(i),
            c if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' => {}
            _ => return None,
        }
    }
    None
}

/// Whether `scheme` is one of the allowed schemes, case-insensitively.
fn matches_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("mailto")
}

/// Escape `&`, `<`, `>` — used when embedding heading text into the ToC HTML.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
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
        out.push_str(&format!(
            "<li><a href=\"#{}\">{}</a>",
            child.id,
            escape_html(&child.text)
        ));
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

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique temporary directory (PID + wall-clock nanos + monotonic counter)
    /// that removes itself on drop: parallel test runs can't collide on a
    /// shared name, and a failed assertion can't leak the directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "web_test_{}_{}_{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock before unix epoch")
                    .as_nanos(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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
        let (html, _) = render_markdown("# Title\n\n- item 1\n- item 2").unwrap();
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
    fn test_split_frontmatter_embedded_pluses() {
        // `+++` inside a TOML value must not close the frontmatter.
        let input = "+++\ntitle = \"C+++\"\n+++\nBody";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.contains("C+++"));
        assert_eq!(md, "Body");
    }

    #[test]
    fn test_split_frontmatter_crlf() {
        let input = "+++\r\ntitle = \"Test\"\r\n+++\r\nBody";
        let (fm, md) = split_frontmatter(input);
        assert!(fm.contains("title = \"Test\""));
        assert_eq!(md, "Body");
    }

    #[test]
    fn test_estimate_reading_time() {
        assert_eq!(
            estimate_reading_time(""),
            1,
            "empty body still counts as 1 min"
        );
        assert_eq!(
            estimate_reading_time("word"),
            1,
            "1 word rounds up to 1 min"
        );
        let body = "word ".repeat(200);
        assert_eq!(estimate_reading_time(&body), 1, "200 words = 1 min");
        let body = "word ".repeat(201);
        assert_eq!(
            estimate_reading_time(&body),
            2,
            "201 words round up to 2 min"
        );
        let body = "word ".repeat(400);
        assert_eq!(
            estimate_reading_time(&body),
            2,
            "400 words ≈ 2 min at 200 wpm"
        );
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
        let (html, _) = render_markdown(md).unwrap();
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_render_markdown_strikethrough() {
        let (html, _) = render_markdown("~~deleted~~").unwrap();
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_render_markdown_code_block() {
        // Ordinary fenced blocks render through pulldown-cmark as pre/code.
        let (html, _) = render_markdown("```\nlet x = 1;\n```").unwrap();
        assert!(
            html.contains("<pre><code>"),
            "fenced block wrapped in pre/code: {html}"
        );
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn test_render_markdown_fenced_language_class() {
        let (html, _) = render_markdown("```rust\nfn main() { let x = 1; }\n```").unwrap();
        // The language id surfaces as pulldown-cmark's language-* class.
        assert!(
            html.contains("class=\"language-rust\""),
            "language class preserved: {html}"
        );
        assert!(html.contains("<pre><code"));
    }

    #[test]
    fn test_render_markdown_code_escaping() {
        // Code content (regardless of language id) must be HTML-escaped.
        let (html, _) = render_markdown("```nope\n<a & b>\n```").unwrap();
        assert!(!html.contains("<a &"), "raw HTML must be escaped: {html}");
        assert!(html.contains("&lt;a"), "`<` escaped: {html}");
        assert!(html.contains("&amp;"), "`&` escaped: {html}");
        assert!(html.contains("&gt;"), "`>` escaped: {html}");
    }

    #[test]
    fn test_render_markdown_tasklist() {
        let (html, _) = render_markdown("- [x] done\n- [ ] todo").unwrap();
        assert!(
            html.contains("task-list") || html.contains("checkbox") || html.contains("disabled")
        );
    }

    #[test]
    fn test_render_markdown_toc_nested() {
        let md = "# Title\n\n## Section A\n\n### Sub one\n\n### Sub two\n\n## Section B";
        let (html, toc) = render_markdown(md).unwrap();
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
        let (_, toc) = render_markdown("just **text**, no headings").unwrap();
        assert!(toc.is_empty());
    }

    #[test]
    fn test_render_markdown_toc_escapes_heading_text() {
        // Heading text is inserted into the ToC with `|safe` in the template,
        // so it must be HTML-escaped at build time to prevent stored XSS from
        // untrusted post content.
        let (_html, toc) = render_markdown("## A & B").unwrap();
        assert!(toc.contains("A &amp; B"), "ToC text must be escaped: {toc}");
        assert!(
            !toc.contains(">A & B<"),
            "raw & must not reach the ToC: {toc}"
        );
    }

    #[test]
    fn test_render_markdown_toc_only_h1() {
        let (html, toc) = render_markdown("# Just a title").unwrap();
        assert!(html.contains("<h1 id=\"just-a-title\">"));
        assert!(toc.is_empty(), "h1-only posts get no ToC");
    }

    #[test]
    fn test_render_markdown_rejects_block_html() {
        let err = render_markdown("<div>raw</div>").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("raw HTML"),
            "block raw HTML must be rejected with a raw-HTML message, got: {msg}"
        );
    }

    #[test]
    fn test_render_markdown_rejects_inline_html() {
        let err = render_markdown("hello <em>world</em>").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("raw HTML"),
            "inline raw HTML must be rejected with a raw-HTML message, got: {msg}"
        );
    }

    #[test]
    fn test_render_markdown_rejects_inline_html_in_heading() {
        // Events inside a heading are consumed separately for the ToC, but must
        // still pass the same validation.
        let err = render_markdown("## hello <em>world</em>").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("raw HTML"),
            "raw HTML inside a heading must be rejected, got: {msg}"
        );
    }

    #[test]
    fn test_render_markdown_rejects_unsafe_destinations() {
        // Table-drive the URL policy boundaries: every snippet must fail
        // rendering with a destination error.
        for (md, label) in [
            ("[x](javascript:alert(1))", "javascript scheme"),
            ("![x](javascript:alert(1))", "javascript scheme image"),
            ("[x](data:foo)", "data scheme"),
            ("[x](file:///etc/passwd)", "file scheme"),
            ("[x](ftp://example.com/x)", "ftp scheme"),
            ("[x](//evil.com)", "protocol-relative"),
            (r"[x](<\evil.com>)", "backslash"),
        ] {
            let err = render_markdown(md).unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains("unsafe link destination"),
                "{label}: expected a destination error for {md:?}, got: {msg}"
            );
        }
    }

    #[test]
    fn test_render_markdown_allows_safer_destinations() {
        let html = render_markdown(
            "[a](https://example.com) [b](http://example.com) [c](mailto:x@y.dev) \
             [d](/relative/path) [e](#fragment) [f](../up) ![g](/images/pic.png)",
        )
        .unwrap()
        .0;
        for needle in [
            r#"href="https://example.com""#,
            r#"href="http://example.com""#,
            r#"href="mailto:x@y.dev""#,
            r#"href="/relative/path""#,
            "href=\"#fragment\"",
            r#"href="../up""#,
            r#"src="/images/pic.png""#,
        ] {
            assert!(html.contains(needle), "missing {needle} in: {html}");
        }
    }

    #[test]
    fn test_validate_event_allows_inline_and_display_math() {
        // Math payloads are safe even though this application never enables
        // `Options::ENABLE_MATH`: they must not fail validation if encountered.
        assert!(validate_event(&Event::InlineMath("$E = mc^2$".into())).is_ok());
        assert!(validate_event(&Event::DisplayMath("$$\\sum x_i$$".into())).is_ok());
    }

    #[test]
    fn test_validate_event_allows_ordinary_events_and_tags() {
        // Leaf events emitted by ordinary enabled markdown.
        for ev in [
            Event::Text("plain text".into()),
            Event::Code("x + 1".into()),
            Event::FootnoteReference("1".into()),
            Event::SoftBreak,
            Event::HardBreak,
            Event::Rule,
            Event::TaskListMarker(true),
            Event::TaskListMarker(false),
        ] {
            assert!(validate_event(&ev).is_ok(), "{ev:?} must be allowed");
        }
        // Start tags for supported CommonMark structures and the enabled
        // table/footnote/strikethrough options.
        for tag in [
            Tag::Paragraph,
            Tag::Heading {
                level: pulldown_cmark::HeadingLevel::H2,
                id: None,
                classes: vec![],
                attrs: vec![],
            },
            Tag::BlockQuote(None),
            Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Indented),
            Tag::List(None),
            Tag::Item,
            Tag::FootnoteDefinition("1".into()),
            Tag::Table(vec![]),
            Tag::TableHead,
            Tag::TableRow,
            Tag::TableCell,
            Tag::Emphasis,
            Tag::Strong,
            Tag::Strikethrough,
        ] {
            assert!(
                validate_event(&Event::Start(tag)).is_ok(),
                "tag must be allowed"
            );
        }
        // Every closing tag for a supported structure (including link/image).
        for end in [
            TagEnd::Paragraph,
            TagEnd::Heading(pulldown_cmark::HeadingLevel::H2),
            TagEnd::BlockQuote(None),
            TagEnd::CodeBlock,
            TagEnd::List(false),
            TagEnd::Item,
            TagEnd::FootnoteDefinition,
            TagEnd::Table,
            TagEnd::TableHead,
            TagEnd::TableRow,
            TagEnd::TableCell,
            TagEnd::Emphasis,
            TagEnd::Strong,
            TagEnd::Strikethrough,
            TagEnd::Link,
            TagEnd::Image,
        ] {
            assert!(
                validate_event(&Event::End(end)).is_ok(),
                "end must be allowed"
            );
        }
        // Safe destinations still pass on start, and their ends pass too.
        for tag in [
            Tag::Link {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url: "/images/a.png".into(),
                title: "".into(),
                id: "".into(),
            },
            Tag::Image {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url: "https://example.com/x".into(),
                title: "".into(),
                id: "".into(),
            },
        ] {
            assert!(
                validate_event(&Event::Start(tag)).is_ok(),
                "safe destination"
            );
        }
        assert!(validate_event(&Event::End(TagEnd::Link)).is_ok());
        assert!(validate_event(&Event::End(TagEnd::Image)).is_ok());
    }

    #[test]
    fn test_validate_event_rejects_raw_html() {
        // Both leaf HTML events and the HTML block start/end tags must fail
        // with the raw-HTML message.
        for ev in [
            Event::Html("<div>".into()),
            Event::InlineHtml("<em>".into()),
            Event::Start(Tag::HtmlBlock),
            Event::End(TagEnd::HtmlBlock),
        ] {
            let msg = format!("{:#}", validate_event(&ev).unwrap_err());
            assert!(
                msg.contains("raw HTML"),
                "{ev:?} must be rejected as raw HTML, got: {msg}"
            );
        }
    }

    #[test]
    fn test_validate_event_rejects_unsafe_destinations() {
        for tag in [
            Tag::Link {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url: "javascript:alert(1)".into(),
                title: "".into(),
                id: "".into(),
            },
            Tag::Link {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url: "//evil.com".into(),
                title: "".into(),
                id: "".into(),
            },
            Tag::Image {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url: "file:///etc/passwd".into(),
                title: "".into(),
                id: "".into(),
            },
        ] {
            let msg = format!("{:#}", validate_event(&Event::Start(tag)).unwrap_err());
            assert!(
                msg.contains("unsafe link destination"),
                "destination must be rejected, got: {msg}"
            );
        }
    }

    #[test]
    fn test_validate_event_rejects_unsupported_structures() {
        // Start tags for structures this application does not enable or
        // support, and their matching closes, must never pass validation.
        for tag in [
            Tag::DefinitionList,
            Tag::DefinitionListTitle,
            Tag::DefinitionListDefinition,
            Tag::Superscript,
            Tag::Subscript,
            Tag::MetadataBlock(pulldown_cmark::MetadataBlockKind::YamlStyle),
        ] {
            let event = Event::Start(tag);
            let label = format!("{event:?}");
            let msg = format!("{:#}", validate_event(&event).unwrap_err());
            assert!(
                msg.contains("unsupported markdown structure"),
                "{label} must be rejected, got: {msg}"
            );
        }
        for end in [
            TagEnd::DefinitionList,
            TagEnd::DefinitionListTitle,
            TagEnd::DefinitionListDefinition,
            TagEnd::Superscript,
            TagEnd::Subscript,
            TagEnd::MetadataBlock(pulldown_cmark::MetadataBlockKind::PlusesStyle),
        ] {
            let msg = format!("{:#}", validate_event(&Event::End(end)).unwrap_err());
            assert!(
                msg.contains("unsupported markdown structure"),
                "{end:?} must be rejected, got: {msg}"
            );
        }
    }

    #[test]
    fn test_validate_destination_accepts_allowed() {
        // Empty, relative, root-relative, fragment, and the allowed schemes
        // (case-insensitively) must all pass.
        for dest in [
            "",
            "relative/path",
            "./x",
            "../x",
            "/rooted",
            "file.html",
            "#fragment",
            "?query=1",
            "http://example.com",
            "HTTPS://EXAMPLE.COM",
            "https://example.com/x",
            "mailto:someone@example.com",
        ] {
            assert!(
                validate_destination(dest).is_ok(),
                "expected '{dest}' to be allowed"
            );
        }
    }

    #[test]
    fn test_validate_destination_rejects_unsafe() {
        // Table-drive the rejected boundary: protocol-relative, whitespace and
        // control characters, and non-allowed explicit schemes.
        for (dest, needle) in [
            ("//evil.com", "protocol-relative"),
            (r"\\evil.com", "backslashes"),
            ("/ /", ""),
            ("a b", ""),
            ("a\tb", ""),
            ("a\u{0001}b", ""),
            ("javascript:alert(1)", "javascript"),
            ("JaVaScRiPt:alert(1)", "JaVaScRiPt"),
            ("data:image/png;base64,x", "data"),
            ("FILE:///x", "FILE"),
            ("ftp://host", "ftp"),
            ("tel:+1555", "tel"),
        ] {
            let err = validate_destination(dest).unwrap_err();
            if !needle.is_empty() {
                assert!(
                    err.contains(needle),
                    "expected '{needle}' in error for {dest:?}, got: {err}"
                );
            } else {
                assert!(
                    err.contains("unsafe link destination"),
                    "expected an unsafe-destination error for {dest:?}, got: {err}"
                );
            }
        }
    }

    #[test]
    fn test_render_markdown_trade_entity() {
        // pulldown-cmark decodes a recognized named entity; the ™ character is
        // not `&,<,>,",'`, so the HTML renderer emits it verbatim.
        let (html, _) = render_markdown("Artificial Intelligence&trade;").unwrap();
        assert!(
            html.contains("Intelligence™"),
            "named entity is decoded, not double-escaped: {html}"
        );
        assert!(
            !html.contains("&trade;"),
            "raw entity must not survive into output: {html}"
        );
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
        let dir = TempDir::new();
        let posts = load_all(dir.path()).unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn test_load_all_empty_dir() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path().join("posts")).unwrap();
        let posts = load_all(dir.path()).unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn test_load_all_reports_policy_violation_filename() {
        let dir = TempDir::new();
        let posts_subdir = dir.path().join("posts");
        std::fs::create_dir_all(&posts_subdir).unwrap();
        std::fs::write(
            posts_subdir.join("unsafe.md"),
            "+++\ntitle = \"Unsafe\"\ndate = \"2024-01-01\"\n+++\n<script>x</script>",
        )
        .unwrap();

        let message = format!("{:#}", load_all(dir.path()).unwrap_err());
        assert!(message.contains("unsafe.md"), "missing filename: {message}");
        assert!(
            message.contains("raw HTML is not allowed"),
            "missing policy failure: {message}"
        );
    }

    #[test]
    fn test_load_all_sorts_by_date_desc() {
        let dir = TempDir::new();
        let posts_subdir = dir.path().join("posts");
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
        let posts = load_all(dir.path()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].slug, "newer", "newest post first");
        assert_eq!(posts[1].slug, "older");
    }

    #[test]
    fn test_load_pages_missing_dir_returns_empty() {
        let dir = TempDir::new();
        let pages = load_pages(dir.path()).unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn test_load_pages_parses_markdown_and_ignores_non_md() {
        let dir = TempDir::new();
        let pages_subdir = dir.path().join("pages");
        std::fs::create_dir_all(&pages_subdir).unwrap();

        std::fs::write(
            pages_subdir.join("about.md"),
            "+++\ntitle = \"About Me\"\n+++\nHello **world**.",
        )
        .unwrap();
        std::fs::write(pages_subdir.join("readme.txt"), "ignore").unwrap();
        let pages = load_pages(dir.path()).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].slug, "about");
        assert_eq!(pages[0].title, "About Me");
        assert!(pages[0].html.contains("<strong>world</strong>"));
    }
}
