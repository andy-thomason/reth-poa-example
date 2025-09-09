use std::sync::Arc;

use reth_ethereum::{cli::Cli, engine::local::{LocalMiner, LocalPayloadAttributesBuilder}, node::{builder::NodeHandle, EthereumNode}};
use tracing::info;

fn main() -> eyre::Result<()> {
    Cli::parse_args()
        .run(|builder, _| async move {
            info!(target: "reth::cli", "Launching node");
            let NodeHandle { node, node_exit_future } =
                builder.node(EthereumNode::default()).launch().await?;

            node.task_executor.spawn_critical("local engine", async move {
                info!(target: "reth::cli", "Using local payload attributes builder for dev mode");

                let blockchain_db = node.provider.clone();
                let chain_spec = node.config.chain.clone();
                let beacon_engine_handle = node.add_ons_handle.beacon_engine_handle.clone();
                let pool = node.pool.clone();
                let payload_builder_handle = node.payload_builder_handle.clone();
    
                // let dev_mining_mode = node.config.dev_mining_mode(pool);
                let dev_mining_mode = node.config.dev_mining_mode(pool);

                let local_payload_attributes_builder = LocalPayloadAttributesBuilder::new(Arc::new(chain_spec.clone()));

                LocalMiner::new(
                    blockchain_db,
                    local_payload_attributes_builder,
                    beacon_engine_handle,
                    dev_mining_mode,
                    payload_builder_handle,
                )
                .run()
                .await
            });
    
            node_exit_future.await
        })
        .unwrap();

    Ok(())
}
