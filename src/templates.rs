use askama::Template;

use crate::posts::{Page, Post};

#[derive(Template)]
#[template(path = "404.html")]
pub struct NotFoundTemplate;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate<'a> {
    pub posts: &'a [Post],
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate<'a> {
    pub post: &'a Post,
    /// Chronologically newer post (earlier in the descending-sorted list).
    pub newer: Option<&'a Post>,
    /// Chronologically older post (later in the descending-sorted list).
    pub older: Option<&'a Post>,
}

#[derive(Template)]
#[template(path = "page.html")]
pub struct PageTemplate<'a> {
    pub page: &'a Page,
}

#[derive(Template)]
#[template(path = "search_results.html")]
pub struct SearchResultsTemplate<'a> {
    pub posts: &'a [&'a Post],
    pub query: &'a str,
}

#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsIndexTemplate<'a> {
    /// Sorted (tag, post-count) pairs.
    pub tags: &'a [(String, usize)],
}

#[derive(Template)]
#[template(path = "tag.html")]
pub struct TagTemplate<'a> {
    pub tag: &'a str,
    pub posts: &'a [&'a Post],
}
