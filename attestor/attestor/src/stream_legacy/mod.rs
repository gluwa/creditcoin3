pub mod cc3;

#[derive(Debug, builder::Builder)]
pub struct Config {
    pub(crate) url_eth: cc_client::secret::RpcUrl,
    pub(crate) url_cc3: cc_client::secret::RpcUrl,
}
