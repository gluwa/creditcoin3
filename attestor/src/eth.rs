use anyhow::Result;
use ethers::providers::{Middleware, Provider, StreamExt, Ws};
use kameo::ActorRef;
use tokio::select;
use tracing::{debug, info};

use crate::attestation::{Attestor, NewBlock};

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(url: &str, attestor: ActorRef<Attestor>) -> Result<()> {
    let provider = Provider::<Ws>::connect(url).await?;
    let mut stream = provider.subscribe_blocks().await?;

    debug!("subscription for new chain heads started...");
    // Kick it off
    loop {
        select! {
            block = stream.next() => match block {
                Some(block) => {
                    info!("New block header: {:?}", block.hash);

                    // Notify the attestor with a new block
                    let _ = attestor.send(NewBlock { block }).await?;
                },
                None => panic!("no block"),
            },
        }
    }
}
