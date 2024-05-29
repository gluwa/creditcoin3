use anyhow::Result;
use kameo::{
    actor::Actor,
    // message::{Context, Message},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info};

use cc_client::Client as CcClient;

pub type Randomness = [u8; 32];

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
pub enum Error {
    #[error("Failed to submit RPC")]
    FailedToSubmit,
    #[error("Failed to sign Babe VRF output")]
    FailedToSignBabeVrf,
    #[error("Failed to check eligibility")]
    FailedToCheckEligibility,
    #[error("Failed to fetch latest digest")]
    FailedToFetchDigest,
    #[error("Invalid attestor")]
    InvalidAttestor,
    #[error("Invalid bls key")]
    InvalidBlsKey,
    #[error("Failed to get cc3 RPC client")]
    FailedToGetRPcClient,
}

#[derive(Debug)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
pub struct Client {
    pub cc_client: CcClient,
    pub nickname: String,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        nickname: impl Into<String> + Clone,
        // private_key: &[u8; 32],
    ) -> Result<Self> {
        let cc_client = CcClient::new(url, key)?;

        Ok(Self {
            cc_client,
            nickname: nickname.into(),
        })
    }

    /// Init the client, this bootstraps registration if not registered already
    pub async fn init(&self) -> Result<()> {
        let is_member = self.cc_client.check_provers_membership().await?;

        if !is_member {
            debug!("Registration in progress... Please wait...");
            self.register().await?;
        }

        info!("Prover ready to start!");

        Ok(())
    }

    /// Register to the prover pallet
    pub async fn register(&self) -> Result<()> {
        self.cc_client.register_prover(self.nickname.clone()).await
    }

    pub async fn start_claim_sub(
        &self,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<()> {
        let mut subscription = self.cc_client.subscribe_claim_submission_events().await?;

        // Process claims in a loop
        loop {
            tokio::select! {
                claim = subscription.next() => {
                    match claim {
                        Some(claim) => {
                            // Process the claim
                            info!("Received a new claim: {:?}", claim);

                            // Handle the claim processing logic here
                        }
                        None => break, // Exit loop if the subscription stream ends
                    }
                }
                rec = &mut cancel => {
                    match rec {
                        Ok(_) => panic!("This doesn't happen"),
                        Err(_) => {
                            info!("Cancellation received, stopping claim processing");
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Actor for Client {}

// // AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
// // It holds the attestation data to be signed by the attestor before submitting
// pub struct AttestationSubmit<H> {
//     pub attestation: AttestationData<H>,
// }

// impl<H> Message<AttestationSubmit<H>> for Client
// where
//     H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug,
// {
//     type Reply = Result<(), Error>;

//     /// Main attestation handler
//     /// This function will check eligibility for submitting attestations if eligible it will sign and submit to cc3
//     async fn handle(
//         &mut self,
//         msg: AttestationSubmit<H>,
//         _ctx: Context<'_, Self, Self::Reply>,
//     ) -> Self::Reply {
//         Ok(())
//     }
// }
