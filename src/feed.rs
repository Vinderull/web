use std::str::FromStr;

use crate::SITE_NAME;
use crate::SITE_URL;
use crate::posts::Post;

/// Convert a `YYYY-MM-DD` date string to an RFC 3339 timestamp at midnight UTC.
fn date_to_fixed_dt(date_str: &str) -> anyhow::Result<atom_syndication::FixedDateTime> {
    let rfc3339 = format!("{date_str}T00:00:00Z");
    atom_syndication::FixedDateTime::from_str(&rfc3339)
        .map_err(|e| anyhow::anyhow!("invalid feed date '{date_str}': {e}"))
}

/// Build an Atom feed from all posts. Pre-rendered once at boot.
pub fn build_feed(posts: &[Post]) -> anyhow::Result<atom_syndication::Feed> {
    let mut feed_builder = atom_syndication::FeedBuilder::default();

    feed_builder
        .title(atom_syndication::Text::plain(SITE_NAME))
        .id(SITE_URL.to_string());

    // Self-link (the feed's own URL)
    feed_builder.link({
        let mut link = atom_syndication::Link::default();
        link.set_href(format!("{SITE_URL}/feed.xml"));
        link.set_rel("self".to_string());
        link
    });

    // Alternate link (the site itself)
    feed_builder.link({
        let mut link = atom_syndication::Link::default();
        link.set_href(SITE_URL.to_string());
        link.set_rel("alternate".to_string());
        link
    });

    let mut entries = Vec::with_capacity(posts.len());
    for post in posts {
        let dt = date_to_fixed_dt(&post.date)?;
        let mut entry_builder = atom_syndication::EntryBuilder::default();

        entry_builder
            .title(atom_syndication::Text::plain(&post.title))
            .id(format!("{SITE_URL}/posts/{}", post.slug))
            .updated(dt)
            .published(dt);

        // Alternate link to the post
        entry_builder.link({
            let mut link = atom_syndication::Link::default();
            link.set_href(format!("{SITE_URL}/posts/{}", post.slug));
            link.set_rel("alternate".to_string());
            link
        });

        // Full content as HTML
        entry_builder.content({
            let mut content = atom_syndication::Content::default();
            content.set_value(Some(post.html.clone()));
            content.set_content_type(Some("html".to_string()));
            content
        });

        // Summary from description
        if let Some(ref desc) = post.description {
            entry_builder.summary(atom_syndication::Text::plain(desc));
        }

        // Tags as categories
        for tag in &post.tags {
            let mut cat = atom_syndication::Category::default();
            cat.set_term(tag);
            entry_builder.category(cat);
        }

        entries.push(entry_builder.build());
    }

    // `updated` = newest post's date (posts are sorted desc, so first is newest)
    if let Some(newest) = posts.first() {
        feed_builder.updated(date_to_fixed_dt(&newest.date)?);
    }

    feed_builder.entries(entries);

    Ok(feed_builder.build())
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
    fn generated_feed_roundtrips_as_valid_atom() {
        let posts = vec![
            fake_post("second", "Second Post", "2025-06-15"),
            fake_post("first", "First Post", "2025-01-01"),
        ];
        let feed = build_feed(&posts).unwrap();
        let xml = feed.to_string();

        let parsed: atom_syndication::Feed = xml.parse().unwrap();

        assert_eq!(parsed.title().value, SITE_NAME);
        assert_eq!(parsed.entries().len(), 2);
        assert_eq!(parsed.entries()[0].title().value, "Second Post");
        assert_eq!(parsed.entries()[1].title().value, "First Post");
    }
}
