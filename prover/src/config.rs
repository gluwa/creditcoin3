#[derive(Debug, Clone)]
/// Server configuration
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
/// - `eth_rpc_url`: Ethereum RPC url
/// - `eth_private_key`: Private key for the ethereum account
/// - `claim_buffer`: The amount of claims we can handle in a certain period
/// - `prover_be_socket_addr`: The prover runs in light mode when this flag is provided. It identifies the web socket address to which proving requests are directed.
pub struct Config {
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub eth_rpc_url: String,
    pub eth_private_key: String,
    pub claim_buffer: u8,
    pub postgres_uri: String,
    pub prover_be_socket_addr: Option<String>,
}
