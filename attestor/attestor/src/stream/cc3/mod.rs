use crate::prelude::*;

mod error;

pub use error::Error;

#[derive(Debug, attestor_macro::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
}

pub struct StreamCC3 {
    api: subxt::OnlineClient<subxt::SubstrateConfig>,
    stream: common::types::SubxtBlockStream,
    chain_key: attestor_primitives::ChainKey,
}

impl StreamCC3 {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use anyhow::Context as _;

        let api = config
            .cc3
            .api()
            .await
            .context("Failed to initialize CC3 client api")?;
        let stream = api
            .blocks()
            .subscribe_finalized()
            .await
            .context("Failed to initialize CC3 finalized block subscription")?;

        anyhow::Ok(Self {
            api,
            stream,
            chain_key: config.chain_key,
        })
    }

    async fn next_block(&mut self) -> Option<Result<common::types::SubxtBlock, Error>> {
        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        loop {
            match self.stream.next().await {
                Some(Ok(block)) => break Some(Ok(block)),
                Some(Err(err)) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve cc3 block, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        break Some(Err(Error::SubxtError(err)));
                    }
                }
                None => match self.api.blocks().subscribe_finalized().await {
                    Ok(stream) => self.stream = stream,
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to reconnect to cc3, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            break Some(Err(Error::SubxtError(err)));
                        }
                    }
                },
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => break None
            }

            delay = (delay * 2).min(DELAY_MAX);
        }
    }
}

impl futures::Stream for StreamCC3 {
    type Item = Result<CC3Events, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::future::Future as _;

        let chain_key = self.chain_key;

        let fut = std::pin::pin!(self.next_block());
        match std::task::ready!(fut.poll(cx)) {
            Some(Ok(block)) => std::task::Poll::Ready(Some(Ok(CC3Events { block, chain_key }))),
            Some(Err(err)) => std::task::Poll::Ready(Some(Err(err))),
            None => std::task::Poll::Ready(None),
        }
    }
}

pub struct CC3Events {
    block: common::types::SubxtBlock,
    chain_key: attestor_primitives::ChainKey,
}

impl CC3Events {
    pub async fn events(
        &self,
    ) -> Result<impl Iterator<Item = Result<cc_client::attestation::CcEvent, Error>>, Error> {
        let events = self.block.events().await.map_err(Error::SubxtError)?;
        let iter = cc_client::Client::extract_events(self.chain_key, &events)
            .map(|event| event.map_err(|err| Error::SubxtError(err.into())));

        Ok(iter)
    }
}
