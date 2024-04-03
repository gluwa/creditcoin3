use anyhow::Result;
use futures::stream::StreamExt;
use kameo::ActorRef;
use tokio::select;
use tracing::{debug, error, info};
use web3::{types::BlockId, Web3};

use crate::attestation::{Attestor, NewBlock};

pub async fn subscribe_to_new_heads(url: &str, attestor: ActorRef<Attestor>) -> Result<()> {
    let ws = web3::transports::WebSocket::new(url).await?;
    let web3 = Web3::new(ws);

    // Subscribe to new block headers
    let mut subscription = web3.eth_subscribe().subscribe_new_heads().await.unwrap();

    debug!("subscription for new chain heads started...");
    // Kick it off
    loop {
        select! {
            header = subscription.next() => match header {
                Some(Ok(header)) => {
                    info!("New block header: {:?}", header.hash);
                    let block_hash = header.hash.unwrap();

                    let block = web3
                        .eth()
                        .block_with_txs(BlockId::Hash(block_hash))
                        .await?.unwrap();

                    // Notify the cc3 client with a new block
                    let _ = attestor.send(NewBlock { block }).await?;
                }
                Some(Err(e)) => {
                    error!("error getting next block: {:?}", e);
                }
                None => panic!("no block"),
            },
        }
    }
}
