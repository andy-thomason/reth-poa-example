use alloy_provider::{network::AnyNetwork, ProviderBuilder};
use reth_ethereum::{
    cli::{chainspec::EthereumChainSpecParser, Cli}, engine::local::LocalPayloadAttributesBuilder, node::{builder::NodeHandle, EthereumNode}
};
use tracing::info;

use crate::{poa_follower::PoaFollower, poa_miner::PoaMiner};

mod poa_miner;
mod poa_follower;

use clap::Parser;

#[derive(Debug, Parser)]
pub struct PoaExampleArgs {
    /// The URL of a producer node. This could be a node with --dev --dev. Used by a follower.
    #[arg(long)]
    pub producer_url: Option<String>,

    /// Where to write the enode of this node.
    #[arg(long)]
    pub enode_file: Option<String>,
}

fn main() -> eyre::Result<()> {
    // let genesis = Genesis::clique_genesis(123123, "0xfD0Fc68A785981772a0ba9581Ed0b1145021C986".parse().unwrap());
    // // pk=0x7ecf2546356e8009833c7c193591628e98661b8b5128ce8bc6b38cd735e19873
    // println!("Suggested genesis:\n{}", serde_json::to_string_pretty(&genesis).unwrap());

    Cli::<EthereumChainSpecParser, PoaExampleArgs>::parse()
        .run(|builder, args| async move {
            info!(target: "reth::cli", "Launching node");
            let NodeHandle { node, node_exit_future } =
                builder.node(EthereumNode::default()).launch().await?;
            
            // Add this enode to --trusted-peers on the follower.
            if let Some(enode_file) = &args.enode_file {
                let enode = reth_ethereum::network::PeersInfo::local_node_record(&node.network);
                std::fs::write(enode_file, enode.to_string())?;
            }

            node.task_executor.spawn_critical("local engine", async move {
                let beacon_engine_handle = node.add_ons_handle.beacon_engine_handle.clone();
                if let Some(producer_url) = args.producer_url.as_ref() {
                    info!(target: "reth::cli", "Running a follower node");
                    let provider = std::sync::Arc::new(ProviderBuilder::default().connect(&producer_url).await.unwrap());

                    PoaFollower::<_, AnyNetwork>::new(
                        beacon_engine_handle,
                        producer_url.clone(),
                        provider
                    )
                    .run()
                    .await
                } else {
                    info!(target: "reth::cli", "Running a producer node");
                    let provider = node.provider.clone();
                    let chain_spec = node.config.chain.clone();
                    let payload_builder_handle = node.payload_builder_handle.clone();
                    let local_payload_attributes_builder = LocalPayloadAttributesBuilder::new(std::sync::Arc::new(chain_spec.clone()));

                    PoaMiner::new(
                        provider,
                        local_payload_attributes_builder,
                        beacon_engine_handle,
                        5,
                        payload_builder_handle,
                    )
                    .run()
                    .await
                }
            });
    
            node_exit_future.await
        })
        .unwrap();

    Ok(())
}
