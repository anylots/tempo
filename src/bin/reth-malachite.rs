use clap::{Args, Parser};
use reth::builder::NodeHandle;
use reth_malachite::{
    app::{node::RethNode, Config, Genesis, State, ValidatorInfo},
    cli::{Cli, MalachiteChainSpecParser},
    consensus::{start_consensus_engine, EngineConfig},
    context::MalachiteContext,
    store::tables::Tables,
    types::Address,
};
use std::{path::PathBuf, sync::Arc};

/// No Additional arguments
#[derive(Debug, Clone, Copy, Default, Args)]
#[non_exhaustive]
struct NoArgs;

fn main() -> eyre::Result<()> {
    reth_cli_util::sigsegv_handler::install();

    // Create the context and initial state
    let ctx = MalachiteContext::default();
    let config = Config::new();

    // Create a genesis with initial validators
    let validator_address = Address::new([1; 20]);
    let validator_info = ValidatorInfo::new(validator_address, 1000, vec![0; 32]);
    let genesis = Genesis::new("1".to_string()).with_validators(vec![validator_info]);

    // Create the node address (in production, derive from public key)
    let address = Address::new([0; 20]);

    Cli::<MalachiteChainSpecParser, NoArgs>::parse().run(|builder, _: NoArgs| async move {
        // Launch the Reth node first to get the engine handle
        let reth_node = RethNode::new();
        let NodeHandle {
            node,
            node_exit_future,
        } = builder
            .node(reth_node)
            .apply(|mut ctx| {
                // Access the database before launch to create tables
                let db = ctx.db_mut();
                if let Err(e) = db.create_tables_for::<Tables>() {
                    tracing::error!("Failed to create consensus tables: {:?}", e);
                } else {
                    tracing::info!("Created consensus tables successfully");
                }
                ctx
            })
            .launch()
            .await?;

        // Get the beacon engine handle
        let app_handle = node.add_ons_handle.beacon_engine_handle.clone();

        // Get the provider from the node
        let provider = node.provider.clone();

        // Create the application state using the factory method
        // This encapsulates Store creation and verification
        let state = State::from_provider(
            ctx.clone(),
            config,
            genesis.clone(),
            address,
            Arc::new(provider),
            app_handle,
        )
        .await?;

        tracing::info!("Application state created successfully");

        // Get the home directory
        let home_dir = PathBuf::from("./data"); // In production, use proper data dir

        // Create Malachite consensus engine configuration
        let engine_config = EngineConfig::new(
            "reth-malachite-1".to_string(),
            "node-0".to_string(),
            "127.0.0.1:26657".parse()?,
        );

        // Start the Malachite consensus engine
        let consensus_handle = start_consensus_engine(state, engine_config, home_dir).await?;

        // Wait for the node to exit
        tokio::select! {
            _ = node_exit_future => {
                tracing::info!("Reth node exited");
            }
            _ = consensus_handle.app => {
                tracing::info!("Consensus engine exited");
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received shutdown signal");
            }
        }

        Ok(())
    })
}
