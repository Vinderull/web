use askama::Template;

use crate::posts::Post;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate<'a> {
    pub posts: &'a [Post],
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate<'a> {
    pub post: &'a Post,
}

#[derive(Template)]
#[template(path = "search_results.html")]
pub struct SearchResultsTemplate<'a> {
    pub posts: &'a [&'a Post],
    pub query: &'a str,
}
