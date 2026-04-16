use super::*;
use user::prelude::*;

pub struct ReconnectingRuntimeApi<'a> {
    client: &'a mut Client,
    runtime_api: subxt::runtime_api::RuntimeApi<
        subxt::SubstrateConfig,
        subxt::OnlineClient<subxt::SubstrateConfig>,
    >,
}

impl<'a> ReconnectingRuntimeApi<'a> {
    pub async fn new(client: &'a mut crate::Client) -> Result<Self, Interrupt<Error>> {
        let runtime_api = match client.api().runtime_api().at_latest().await {
            Ok(runtime_api) => runtime_api,
            Err(err) => {
                let (client_new, runtime_api) = reconnect(client, err).await?;
                *client = client_new;
                runtime_api
            }
        };

        Ok(Self {
            client,
            runtime_api,
        })
    }

    pub async fn call<Call: subxt::runtime_api::Payload>(
        &mut self,
        call: impl Fn() -> Call,
    ) -> Result<Call::ReturnType, Interrupt<Error>> {
        loop {
            match self.runtime_api.call(call()).await {
                Ok(res) => break Ok(res),
                Err(err) => {
                    let (client, runtime_api) = reconnect(self.client, err).await?;
                    *self.client = client;
                    self.runtime_api = runtime_api;
                }
            }
        }
    }
}

async fn reconnect(
    client: &Client,
    err: subxt::Error,
) -> Result<
    (
        Client,
        subxt::runtime_api::RuntimeApi<
            subxt::SubstrateConfig,
            subxt::OnlineClient<subxt::SubstrateConfig>,
        >,
    ),
    Interrupt<Error>,
> {
    tracing::warn!(?err, "CC3 connection lost");

    let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
        .max_delay(std::time::Duration::from_millis(5_000))
        .map(tokio_retry::strategy::jitter);
    let reconnect = || {
        tracing::warn!("Reconnecting to CC3...");

        let mut cc3 = client.clone();
        async move {
            cc3.reconnect().await.map_err(|err| {
                tracing::error!(?err, "Failed to reconnect to CC3");
                err
            })?;

            let runtime_api = cc3.api().runtime_api().at_latest().await.map_err(|err| {
                tracing::error!(?err, "Failed to reconnect to CC3");
                Error::SubxtError(err)
            })?;

            Ok::<_, Error>((cc3, runtime_api))
        }
    };
    tokio::select! {
        retry = tokio_retry::Retry::spawn(strategy, reconnect) => {
            Ok(retry.expect("Unbounded retry cannot error"))
        }
        _ = tokio::signal::ctrl_c() => {
            Err(Interrupt::Stop)
        }
    }
}
