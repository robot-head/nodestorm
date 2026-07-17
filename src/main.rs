use anyhow::Context;
use clap::Parser;
use nodestorm::cli::Cli;
use nodestorm::store::Store;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let session_path = cli.session_path()?;
    let store = if cli.demo {
        Store::with_doc(nodestorm::demo::demo_doc())
    } else {
        match nodestorm::persist::load(&session_path) {
            Some(state) => Store::new(state),
            None => Store::with_doc(Default::default()),
        }
    };

    // The MCP server gets its own runtime on a dedicated thread; the UI
    // (Dioxus/tao) owns the main thread. Bind before launching the window so
    // a taken port fails fast with a clear message.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("mcp-worker")
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    let listener = runtime.block_on(nodestorm::server::bind(cli.port))?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    runtime.spawn(nodestorm::persist::autosave_task(
        store.clone(),
        session_path.clone(),
    ));

    let server_thread = std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn({
            let store = store.clone();
            move || {
                if let Err(err) =
                    runtime.block_on(nodestorm::server::serve(listener, store, shutdown_rx))
                {
                    tracing::error!(error = ?err, "mcp server exited");
                }
            }
        })?;

    tracing::info!(
        port = cli.port,
        session = %session_path.display(),
        "nodestorm up — agents connect via http://127.0.0.1:{}/mcp",
        cli.port
    );

    if cli.headless {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(tokio::signal::ctrl_c())
            .context("waiting for ctrl-c")?;
        tracing::info!("ctrl-c — shutting down");
    } else {
        nodestorm::ui::launch(store.clone(), cli);
    }

    // Final save (the debounced autosave may not have caught the last change).
    if let Err(err) = nodestorm::persist::save(&session_path, &store.snapshot_state()) {
        tracing::warn!(%err, "final save failed");
    }
    let _ = shutdown_tx.send(true);
    let _ = server_thread.join();
    Ok(())
}
