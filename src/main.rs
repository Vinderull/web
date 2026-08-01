use axum::Router;
use web::{build_app, config::Config, posts, sandbox};

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

    // Load standalone pages alongside posts, before sandboxing (landlock
    // denies reads outside static/ once applied).
    let about_page = posts::load_pages(&config.content_dir)?
        .into_iter()
        .find(|p| p.slug == "about");

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

        let app: Router = build_app(loaded_posts, about_page, &config.static_dir)?;

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
