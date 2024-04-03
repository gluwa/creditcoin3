use futures::stream::StreamExt;
use jsonrpsee_core::client::ClientT;
use jsonrpsee_core::rpc_params;
use jsonrpsee_http_client::HttpClientBuilder;
use nanorand::Rng;
use std::error::Error;
use subxt::{OnlineClient, SubstrateConfig};
use tokio::select;
use tracing_subscriber::EnvFilter;
use web3::Web3;

use creditcoin3_attestor_gossip::{Attestation, AttestorId, Topic};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    // Replace this URL with your local Ethereum node's JSON-RPC URL
    let ws = web3::transports::WebSocket::new("ws://localhost:8545").await?;
    let web3 = Web3::new(ws);

    // Subscribe to new block headers
    let mut subscription = web3.eth_subscribe().subscribe_new_heads().await.unwrap();

    let mut rng = nanorand::tls_rng();
    let attestor_id = AttestorId::new(rng.generate::<u64>());

    let rpc_client = HttpClientBuilder::new().build("http://localhost:9944")?;

    // Kick it off
    loop {
        select! {
            header = subscription.next() => match header {
                Some(Ok(header)) => {
                    println!("New block header: {:?}", header.hash);
                    let block_hash = header.hash.unwrap();

                    // let block: Option<Block<_>> = web3
                    //     .eth()
                    //     .block_with_txs(BlockId::Hash(block_hash))
                    //     .await
                    //     .unwrap();

                    let attestation = Attestation {
                        round: 1,
                        header_number: header.number.unwrap().as_u64(),
                        attestor: attestor_id.clone(),
                        header_hash: block_hash,
                        topic: Topic::new(1),
                    };

                    let res = rpc_client.request("attestor_submitAttestation", rpc_params!(attestation)).await?;
                    println!("rpc req sent: {:?}", res);

                }
                Some(Err(e)) => {
                    eprintln!("{:?}", e);
                }
                None => panic!("no block"),
            },
        }
    }
}

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub type Randomness = [u8; 32];

pub(crate) async fn _fetch_babe_randomness() -> Result<Option<Randomness>, subxt::Error> {
    let api = OnlineClient::<SubstrateConfig>::new().await?;

    let storage_query = cc3::storage().babe().randomness();

    // Probably want to get it from 2 epochs ago (need to fetch current epoch and epoch duration for that)
    let result = api
        .storage()
        .at_latest()
        .await?
        .fetch(&storage_query)
        .await?;

    Ok(result)
}
