pub mod cc3;

#[derive(Debug, builder::Builder)]
pub struct Config {
    pub(crate) url_eth: url::Url,
    pub(crate) url_cc3: url::Url,
}
