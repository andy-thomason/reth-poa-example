//! Proof of Authority Miner based on LocalMiner.

use std::{
    collections::VecDeque,
    time::{Duration, UNIX_EPOCH},
};

use alloy_primitives::B256;
use eyre::OptionExt;
use reth_ethereum::{
    node::api::{
        BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion, PayloadAttributesBuilder,
        PayloadTypes,
    },
    rpc::types::engine::ForkchoiceState,
    storage::BlockReader,
};
use reth_payload_builder::{PayloadBuilderHandle, PayloadKind};
use tracing::{error, info};

#[derive(Debug)]
pub struct PoaMiner<T: PayloadTypes, B> {
    /// The payload attribute builder for the engine
    payload_attributes_builder: B,
    /// Sender for events to engine.
    to_engine: ConsensusEngineHandle<T>,
    /// The block rate in seconds.
    interval: u64,
    /// The payload builder for the engine
    payload_builder: PayloadBuilderHandle<T>,
    /// Timestamp for the next block.
    last_timestamp: u64,
    /// Stores latest mined blocks.
    last_block_hashes: VecDeque<B256>,
}

impl<T: PayloadTypes, B> PoaMiner<T, B>
where
    T: PayloadTypes,
    B: PayloadAttributesBuilder<<T as PayloadTypes>::PayloadAttributes>,
{
    pub fn new(
        provider: impl BlockReader,
        payload_attributes_builder: B,
        to_engine: ConsensusEngineHandle<T>,
        interval: u64,
        payload_builder: PayloadBuilderHandle<T>,
    ) -> Self {
        info!(
            "PoaMiner: Starting POA at block {}",
            provider.best_block_number().unwrap()
        );

        let latest_header = provider
            .sealed_header(provider.best_block_number().unwrap())
            .unwrap()
            .unwrap();
        let last_block_hashes = VecDeque::from([latest_header.hash()]);
        let last_timestamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("cannot be earlier than UNIX_EPOCH")
            .as_secs();
        Self {
            payload_attributes_builder,
            to_engine,
            interval,
            payload_builder,
            last_timestamp,
            last_block_hashes,
        }
    }

    pub async fn run(mut self) {
        let mut fcu_interval = tokio::time::interval(Duration::from_secs(1));
        let mut block_interval = tokio::time::interval(Duration::from_secs(self.interval));
        loop {
            tokio::select! {
                // Wait for the interval or the pool to receive a transaction
                // Note that this should be more like the original MiningMode.
                _ = block_interval.tick() => {
                    if let Err(e) = self.advance().await {
                        error!(target: "engine::local", "Error advancing the chain: {:?}", e);
                    }
                }
                // send FCU once in a while
                _ = fcu_interval.tick() => {
                    if let Err(e) = self.update_forkchoice_state().await {
                        error!(target: "engine::local", "Error updating fork choice: {:?}", e);
                    }
                }
            }
        }
    }

    /// Sends a FCU to the engine.
    async fn update_forkchoice_state(&self) -> eyre::Result<()> {
        let res = self
            .to_engine
            .fork_choice_updated(
                self.forkchoice_state(),
                None,
                EngineApiMessageVersion::default(),
            )
            .await?;

        if !res.is_valid() {
            eyre::bail!("Invalid fork choice update")
        }

        Ok(())
    }

    /// Returns current forkchoice state.
    fn forkchoice_state(&self) -> ForkchoiceState {
        ForkchoiceState {
            head_block_hash: *self
                .last_block_hashes
                .back()
                .expect("at least 1 block exists"),
            safe_block_hash: *self
                .last_block_hashes
                .get(self.last_block_hashes.len().saturating_sub(32))
                .expect("at least 1 block exists"),
            finalized_block_hash: *self
                .last_block_hashes
                .get(self.last_block_hashes.len().saturating_sub(64))
                .expect("at least 1 block exists"),
        }
    }

    /// Generates payload attributes for a new block, passes them to FCU and inserts built payload
    /// through newPayload.
    async fn advance(&mut self) -> eyre::Result<()> {
        let timestamp = std::cmp::max(
            self.last_timestamp + 1,
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("cannot be earlier than UNIX_EPOCH")
                .as_secs(),
        );

        let res = self
            .to_engine
            .fork_choice_updated(
                self.forkchoice_state(),
                Some(self.payload_attributes_builder.build(timestamp)),
                EngineApiMessageVersion::default(),
            )
            .await?;

        if !res.is_valid() {
            eyre::bail!("Invalid payload status")
        }

        let payload_id = res.payload_id.ok_or_eyre("No payload id")?;

        let Some(Ok(payload)) = self
            .payload_builder
            .resolve_kind(payload_id, PayloadKind::WaitForPending)
            .await
        else {
            eyre::bail!("No payload")
        };

        let block = payload.block();

        let payload = T::block_to_payload(block.clone());
        let res = self.to_engine.new_payload(payload).await?;

        if !res.is_valid() {
            eyre::bail!("Invalid payload")
        }

        self.last_timestamp = timestamp;
        self.last_block_hashes.push_back(block.hash());
        // ensure we keep at most 64 blocks
        if self.last_block_hashes.len() > 64 {
            self.last_block_hashes.pop_front();
        }

        Ok(())
    }
}
