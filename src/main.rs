use anyhow::Context;
use clap::Parser;
use nodestorm::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let sessions = nodestorm::sessions::Sessions::open_from_cli(&cli)?;
    let terminals = nodestorm::terminal::TerminalManager::new();
    if cli.demo {
        sessions
            .active_store()
            .apply_propose(nodestorm::demo::demo_doc())
            .context("loading the demo graph")?;
    } else if let Some(n) = cli.demo_big {
        sessions
            .active_store()
            .apply_propose(nodestorm::demo::big_doc(n))
            .context("loading the big demo graph")?;
    }

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

    sessions.spawn_autosaves(runtime.handle());

    let server_thread = std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn({
            let sessions = sessions.clone();
            let terminals = terminals.clone();
            move || {
                if let Err(err) = runtime.block_on(nodestorm::server::serve(
                    listener,
                    sessions,
                    terminals,
                    shutdown_rx,
                )) {
                    tracing::error!(error = ?err, "mcp server exited");
                }
            }
        })?;

    tracing::info!(
        port = cli.port,
        active = %sessions.active_name(),
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
        nodestorm::ui::launch(sessions.clone(), terminals.clone(), cli);
    }

    // The UI (or ctrl-c) is done: no PTY child outlives the app.
    terminals.kill_all();

    // Final saves (the debounced autosaves may not have caught the last
    // change in every session).
    sessions.save_all();
    let _ = shutdown_tx.send(true);
    let _ = server_thread.join();
    Ok(())
}
