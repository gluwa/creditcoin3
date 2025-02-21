#[derive(Debug, Clone)]
/// Server configuration
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
/// - `eth_rpc_url`: Ethereum RPC url
/// - `cc3_evm_private_key`: Private key for a creditcoin3 EVM account
/// - `cost_per_bytes`: Per byte cost of proving a query
/// - `base_fee`: Base fee that prover requires to start processing any query
/// - `claim_buffer`: The amount of claims we can handle in a certain period
/// - `prover_be_socket_addr`: The prover runs in light mode when this flag is provided. It identifies the web socket address to which proving requests are directed.
/// - `be_api_key`: When in light mode, the prover sends proof requests to the prover BE. The BE requires an api key for auth. This takes the form of a UUID.
pub struct Config {
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub eth_rpc_url: String,
    pub cc3_evm_private_key: String,
    pub cost_per_byte: u64,
    pub base_fee: u64,
    pub claim_buffer: u8,
    pub postgres_uri: String,
    pub prover_be_socket_addr: Option<String>,
    pub be_api_key: Option<String>,
}
