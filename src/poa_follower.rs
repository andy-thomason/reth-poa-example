use std::sync::Arc;

use futures_util::StreamExt;
use reth_ethereum::{node::{api::{BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion, ExecutionPayload, PayloadTypes}}, primitives::{AlloyBlockHeader, NodePrimitives, SealedBlock}, rpc::types::engine::ForkchoiceState};
use alloy_provider::{Network, Provider};
use tracing::{info, warn};
use serde_json::Value;


pub struct PoaFollower<T: PayloadTypes, N: Network> {
    /// Sender for events to engine.
    to_engine: ConsensusEngineHandle<T>,

    /// ws:// URL of producer node.
    producer_url: String,

    provider: Arc<dyn Provider<N>>,
}

/// Recursively rename "uncles" fields to "ommers" in JSON data
fn rename_uncles_to_ommers(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            // Check if there's an "uncles" field and rename it to "ommers"
            if let Some(uncles_value) = map.remove("uncles") {
                map.insert("ommers".to_string(), uncles_value);
            }
            
            // Recursively process nested objects
            for (_, v) in map.iter_mut() {
                *v = rename_uncles_to_ommers(v.clone());
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                *item = rename_uncles_to_ommers(item.clone());
            }
        }
        _ => {}
    }
    value
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

                    // Rename "uncles" fields to "ommers" in the JSON
                    let json = rename_uncles_to_ommers(json);

                    info!("json: {}", json);
                    let header = serde_json::from_value::<B::Header>(json.clone())
                        .expect("Header deserialization cannot fail");
                    let body = serde_json::from_value::<B::Body>(json)
                        .expect("Body deserialization cannot fail");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_rename_uncles_to_ommers() {
        // Test basic rename
        let input = json!({
            "number": "0x1",
            "hash": "0x123",
            "uncles": ["0xabc", "0xdef"]
        });
        
        let result = rename_uncles_to_ommers(input);
        
        assert!(result.get("ommers").is_some());
        assert!(result.get("uncles").is_none());
        assert_eq!(result["ommers"], json!(["0xabc", "0xdef"]));
    }

    #[test]
    fn test_rename_uncles_nested() {
        // Test nested rename
        let input = json!({
            "header": {
                "number": "0x1",
                "uncles": ["0x111"]
            },
            "body": {
                "transactions": [],
                "uncles": ["0x222", "0x333"]
            },
            "uncles": ["0x444"]
        });
        
        let result = rename_uncles_to_ommers(input);
        
        // Check top level
        assert!(result.get("ommers").is_some());
        assert!(result.get("uncles").is_none());
        assert_eq!(result["ommers"], json!(["0x444"]));
        
        // Check nested in header
        assert!(result["header"].get("ommers").is_some());
        assert!(result["header"].get("uncles").is_none());
        assert_eq!(result["header"]["ommers"], json!(["0x111"]));
        
        // Check nested in body
        assert!(result["body"].get("ommers").is_some());
        assert!(result["body"].get("uncles").is_none());
        assert_eq!(result["body"]["ommers"], json!(["0x222", "0x333"]));
    }

    #[test]
    fn test_rename_uncles_array() {
        // Test rename in arrays
        let input = json!([
            {
                "number": "0x1",
                "uncles": ["0xaaa"]
            },
            {
                "number": "0x2",
                "uncles": ["0xbbb"]
            }
        ]);
        
        let result = rename_uncles_to_ommers(input);
        
        let array = result.as_array().unwrap();
        assert!(array[0].get("ommers").is_some());
        assert!(array[0].get("uncles").is_none());
        assert!(array[1].get("ommers").is_some());
        assert!(array[1].get("uncles").is_none());
    }

    #[test]
    fn test_no_uncles_field() {
        // Test that objects without uncles field are unchanged
        let input = json!({
            "number": "0x1",
            "hash": "0x123",
            "transactions": []
        });
        
        let expected = input.clone();
        let result = rename_uncles_to_ommers(input);
        
        assert_eq!(result, expected);
    }
}


