use std::sync::Arc;

use futures_util::StreamExt;
use reth_ethereum::{node::{api::{BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion, ExecutionPayload, FullNodeComponents, FullNodeTypes, NodeTypes, PayloadTypes}, builder::DebugNode}, primitives::{AlloyBlockHeader, NodePrimitives, SealedBlock}, rpc::types::engine::ForkchoiceState};
use alloy_provider::{network::BlockResponse, Network, Provider};
use tracing::{info, warn};


pub struct PoaFollower<T: PayloadTypes, N: Network> {
    /// Sender for events to engine.
    to_engine: ConsensusEngineHandle<T>,

    /// ws:// URL of producer node.
    producer_url: String,

    provider: Arc<dyn Provider<N>>,
}

impl<B : reth_ethereum::primitives::Block, T: PayloadTypes, N: Network> PoaFollower<T, N>
where
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = B>>>,

{
    pub fn new(to_engine: ConsensusEngineHandle<T>, producer_url: String, provider: Arc<dyn Provider<N>>) -> Self {
        Self { to_engine, producer_url, provider }
    }


    pub async fn run(self) {

        let mut stream = match self.provider.subscribe_blocks().await {
            Ok(sub) => sub.into_stream(),
            Err(err) => {
                warn!(
                    target: "consensus::debug-client",
                    %err,
                    url=%self.producer_url,
                    "Failed to subscribe to blocks",
                );
                return;
            }
        };
        while let Some(header) = stream.next().await {
            info!("block {}", header.number());
            let block = self
                .provider
                .get_block_by_number(header.number().into())
                .full()
                .await.unwrap()
                .ok_or_else(|| eyre::eyre!("block not found by number {}", header.number()));

            match block {
                Ok(block_response) => {
                    let json = serde_json::to_value(block_response)
                        .expect("Block serialization cannot fail");

                    info!("json: {}", json);
                    // let transactions = json.get("transactions").unwrap().clone();
                    let header = serde_json::from_value::<B::Header>(json.clone())
                        .expect("Header deserialization cannot fail");
                    // let body = serde_json::from_value::<B::Body>(json)
                    //     .expect("Body deserialization cannot fail");
                    let body = Default::default();
                    let pblock = B::new(header, body);

                    let payload = T::block_to_payload(SealedBlock::new_unhashed(pblock));

                    let hash = payload.block_hash();
                    
                    println!("{payload:?}");
                    let _ = self.to_engine.new_payload(payload).await;

                    let fcu = ForkchoiceState {
                        head_block_hash: hash,
                        safe_block_hash: hash,
                        finalized_block_hash: hash,
                    };

                    let _ = self.to_engine.fork_choice_updated(fcu, None, EngineApiMessageVersion::default()).await;

                }
                Err(err) => {
                    warn!(
                        target: "consensus::debug-client",
                        %err,
                        url=%self.producer_url,
                        "Failed to fetch a block",
                    );
                }
            }
        }
    }

    // async fn get_block(&self, block_number: u64) -> eyre::Result<N::BlockResponse> {
    //     let block = self
    //         .provider
    //         .get_block_by_number(block_number.into())
    //         .full()
    //         .await?
    //         .ok_or_else(|| eyre::eyre!("block not found by number {}", block_number))?;
    //     // Ok((self.convert)(block))
    //     Ok(block)
    // }
}


