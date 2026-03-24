//! BACnet gateway binary entry point.
//!
//! Loads TOML config, applies CLI overrides, starts the BACnet stack,
//! and serves the HTTP REST API + MCP server.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::Router;
use clap::Parser;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio_util::sync::CancellationToken;

use bacnet_gateway::api::api_router;
use bacnet_gateway::auth::bearer::BearerTokenAuth;
use bacnet_gateway::auth::Authenticator;
use bacnet_gateway::builder::GatewayBuilder;
use bacnet_gateway::config::GatewayConfig;
use bacnet_gateway::mcp::GatewayMcp;

/// BACnet HTTP REST API and MCP server gateway.
#[derive(Parser)]
#[command(name = "bacnet-gateway", about = "BACnet HTTP/MCP gateway")]
struct Cli {
    /// Config file path.
    #[arg(short, long, default_value = "gateway.toml")]
    config: String,

    /// Override server bind address.
    #[arg(short, long)]
    bind: Option<String>,

    /// Override API key (or set BACNET_GATEWAY_API_KEY env var).
    #[arg(short = 'k', long)]
    api_key: Option<String>,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress all output except errors.
    #[arg(short, long)]
    quiet: bool,

    /// Disable MCP endpoint.
    #[arg(long)]
    no_mcp: bool,

    /// Disable REST API (MCP only).
    #[arg(long)]
    no_api: bool,

    /// Read-only mode — disable all write operations.
    #[arg(long)]
    read_only: bool,

    /// Print resolved config and exit.
    #[arg(long)]
    print_config: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize tracing.
    let level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .init();

    // Load config.
    let config_text = match std::fs::read_to_string(&cli.config) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Config file '{}' not found.", cli.config);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error reading config file '{}': {e}", cli.config);
            std::process::exit(1);
        }
    };

    let mut config = GatewayConfig::from_toml(&config_text)?;

    // Apply CLI overrides.
    if let Some(bind) = &cli.bind {
        config.server.bind = bind.clone();
    }
    if let Some(key) = &cli.api_key {
        config.server.api_key = Some(key.clone());
    }
    if config.server.api_key.is_none() {
        if let Ok(key) = std::env::var("BACNET_GATEWAY_API_KEY") {
            config.server.api_key = Some(key);
        }
    }
    if cli.read_only {
        config.server.read_only = true;
    }

    // Validate.
    if cli.no_mcp && cli.no_api {
        eprintln!("Error: --no-mcp and --no-api cannot both be set.");
        std::process::exit(1);
    }
    config.validate().map_err(|e| {
        eprintln!("Config validation error: {e}");
        e
    })?;

    if cli.print_config {
        println!("{config:#?}");
        return Ok(());
    }

    let bind_addr: SocketAddr = config.server.bind.parse().map_err(|e| {
        eprintln!("Invalid bind address '{}': {e}", config.server.bind);
        e
    })?;

    tracing::info!("Starting BACnet gateway...");

    // Build the BACnet stack.
    let built = GatewayBuilder::new(config.clone())
        .build()
        .await
        .map_err(|e| {
            eprintln!("Failed to build gateway: {e}");
            e
        })?;

    tracing::info!("BACnet server started on MAC {:02x?}", built.server_mac);

    // Build the Axum router.
    let mut router = Router::new();

    if !cli.no_api {
        let auth: Option<Box<dyn Authenticator>> = config
            .server
            .api_key
            .as_ref()
            .map(|key| Box::new(BearerTokenAuth::new(key.clone())) as Box<dyn Authenticator>);
        let api = api_router(built.state.clone(), auth);
        router = router.merge(api);
        tracing::info!("REST API enabled at /api/v1/");
    }

    if !cli.no_mcp {
        let ct = CancellationToken::new();
        let mcp_state = built.state.clone();
        let mcp_service = StreamableHttpService::new(
            move || Ok(GatewayMcp::new(mcp_state.clone())),
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig {
                cancellation_token: ct.child_token(),
                ..Default::default()
            },
        );

        let mut mcp_router = Router::new().nest_service("/mcp", mcp_service);

        // Apply auth to MCP endpoint if configured.
        if let Some(api_key) = &config.server.api_key {
            let authenticator = Arc::new(BearerTokenAuth::new(api_key.clone()));
            mcp_router = mcp_router.layer(middleware::from_fn(move |req: Request, next: Next| {
                let auth = authenticator.clone();
                async move {
                    match auth.authenticate(req.headers()) {
                        Ok(()) => next.run(req).await,
                        Err(e) => (
                            e.status,
                            axum::Json(serde_json::json!({ "error": e.message })),
                        )
                            .into_response(),
                    }
                }
            }));
        }

        router = router.merge(mcp_router);
        tracing::info!("MCP server enabled at /mcp");
    }

    tracing::info!("Listening on {bind_addr}");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Shutting down...");
    drop(built);

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
    tracing::info!("Received shutdown signal");
}
