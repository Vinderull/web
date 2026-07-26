mod config;
mod posts;
mod sandbox;
mod templates;

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use askama::Template;
use config::Config;
use templates::{IndexTemplate, PostTemplate};

#[derive(Clone)]
struct AppState {
    posts: Arc<Vec<posts::Post>>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!("Content dir: {}", config.content_dir.display());
    tracing::info!("Static dir: {}", config.static_dir.display());

    // Load posts from disk before sandboxing
    let loaded_posts = posts::load_all(&config.content_dir)?;
    tracing::info!("Loaded {} posts", loaded_posts.len());

    // Bind listener before applying landlock (landlock would block bind on V4+)
    let listener = std::net::TcpListener::bind(&config.bind_addr)?;
    listener.set_nonblocking(true)?;
    tracing::info!("Listening on {}", config.bind_addr);

    // Apply landlock sandbox before starting tokio runtime so worker threads
    // inherit the landlock domain.
    sandbox::apply(&config.static_dir)?;

    // Create runtime after sandboxing so worker threads inherit restrictions
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let listener = tokio::net::TcpListener::from_std(listener)?;

        let state = AppState {
            posts: Arc::new(loaded_posts),
        };

        let app = Router::new()
            .route("/", get(index))
            .route("/posts/{slug}", get(post))
            .nest_service("/static", ServeDir::new(&config.static_dir))
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        let shutdown = async {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            let sigterm = async {
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("install SIGTERM handler")
                    .recv()
                    .await;
            };
            #[cfg(not(unix))]
            let sigterm = std::future::pending::<()>();

            tokio::select! {
                _ = ctrl_c => {}
                _ = sigterm => {}
            }
            tracing::info!("Shutdown signal received");
        };

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
}

async fn index(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let template = IndexTemplate {
        posts: &state.posts,
    };
    let html = template
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(html))
}

async fn post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let p = state
        .posts
        .iter()
        .find(|p| p.slug == slug)
        .ok_or(StatusCode::NOT_FOUND)?;
    let template = PostTemplate { post: p };
    let html = template
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(html))
}
