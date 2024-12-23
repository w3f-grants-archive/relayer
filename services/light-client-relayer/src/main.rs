//! A block poller using the `webb-relayer` framework.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use tokio::signal::unix;
use webb_light_client_relayer::start_light_client_service;

use webb_relayer_config::{
    block_poller::BlockPollerConfig,
    cli::{create_store, load_config, setup_logger, Opts},
};
use webb_relayer_context::RelayerContext;

/// Starts all background services for all chains configured in the config file.
///
/// Returns a future that resolves when all services are started successfully.
///
/// # Arguments
///
/// * `ctx` - RelayContext reference that holds the configuration
/// * `store` -[Sled](https://sled.rs)-based database store
pub async fn ignite(ctx: &RelayerContext) -> anyhow::Result<()> {
    tracing::debug!(
        "Relayer configuration: {}",
        serde_json::to_string_pretty(&ctx.config)?
    );

    // now we go through each chain, in our configuration
    for chain_config in ctx.config.eth2.values() {
        if !chain_config.enabled {
            continue;
        }

        let mut chain_config = chain_config.clone();
        let api_key_string = std::env::var("ETH1_INFURA_API_KEY")?;
        chain_config.eth1_endpoint = chain_config
            .eth1_endpoint
            .replace("ETH1_INFURA_API_KEY", &api_key_string);

        let chain_name = &chain_config.name;
        let poller_config = BlockPollerConfig::default();
        tracing::debug!(
            "Starting Background Services for ({}) chain ({:?})",
            chain_name,
            poller_config
        );

        tracing::debug!("Starting light client relay ({:#?})", poller_config,);
        start_light_client_service(ctx, chain_config)?;
    }
    Ok(())
}

/// The main entry point for the relayer.
///
/// # Arguments
///
/// * `args` - The command line arguments.
#[paw::main]
#[tokio::main(flavor = "multi_thread")]
async fn main(args: Opts) -> anyhow::Result<()> {
    setup_logger(args.verbose, "webb_light_client_relayer")?;
    match dotenv::dotenv() {
        Ok(_) => {
            tracing::trace!("Loaded .env file");
        }
        Err(e) => {
            tracing::warn!("Failed to load .env file: {}", e);
        }
    }

    // The configuration is validated and configured from the given directory
    let config = load_config(args.config_dir.clone())?;
    tracing::trace!("Loaded config.. {:#?}", config);
    // Persistent storage for the relayer
    let store = create_store(&args).await?;
    // The RelayerContext takes a configuration, and populates objects that are needed
    // throughout the lifetime of the relayer. Items such as wallets and providers, as well
    // as a convenient place to access the configuration.
    let ctx = RelayerContext::new(config, store)?;
    tracing::trace!("Created persistent storage..");
    // The build_web_relayer command sets up routing (endpoint queries / requests mapped to handled code)
    // so clients can interact with the relayer
    let server_handle =
        tokio::spawn(webb_relayer::service::build_web_services(ctx.clone()));

    // Start all background services.
    // This does not block, as it will fire the services on background tasks.
    ignite(&ctx).await?;
    tracing::event!(
        target: webb_relayer_utils::probe::TARGET,
        tracing::Level::DEBUG,
        kind = %webb_relayer_utils::probe::Kind::Lifecycle,
        started = true
    );
    // watch for signals
    let mut ctrlc_signal = unix::signal(unix::SignalKind::interrupt())?;
    let mut termination_signal = unix::signal(unix::SignalKind::terminate())?;
    let mut quit_signal = unix::signal(unix::SignalKind::quit())?;
    let shutdown = || {
        tracing::event!(
            target: webb_relayer_utils::probe::TARGET,
            tracing::Level::DEBUG,
            kind = %webb_relayer_utils::probe::Kind::Lifecycle,
            shutdown = true
        );
        tracing::warn!("Shutting down...");
        // send shutdown signal to all of the application.
        ctx.shutdown();
        std::thread::sleep(std::time::Duration::from_millis(300));
        tracing::info!("Clean Exit ..");
    };
    tokio::select! {
        _ = ctrlc_signal.recv() => {
            tracing::warn!("Interrupted (Ctrl+C) ...");
            shutdown();
        },
        _ = termination_signal.recv() => {
            tracing::warn!("Got Terminate signal ...");
            shutdown();
        },
        _ = quit_signal.recv() => {
            tracing::warn!("Quitting ...");
            shutdown();
        },
        _ = server_handle => {
            tracing::warn!("Relayer axum server stopped");
            shutdown();
        }
    }
    Ok(())
}
