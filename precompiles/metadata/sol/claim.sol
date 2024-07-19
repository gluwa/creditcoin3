// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The precompiled address of the ClaimContract on the Ethereum network.
address constant CLAIM_CONTRACT_ADDRESS = 0x0000000000000000000000000000000000000Be9;

/// @dev Instance of the ClaimContract interface at the precompiled address.
ClaimContract constant CLAIM_CONTRACT_ADRRESS = ClaimContract(CLAIM_CONTRACT_ADDRESS);

/// @title ClaimContract interface
/// @notice This interface defines the functions and events for interacting with the ClaimContract.
interface ClaimContract {
    struct FeltRange {
        uint32 start;
        uint32 end;
    }

    /// @notice Submit a claim to the contract.
    /// @param chain_id The ID of the blockchain where the claim originates.
    /// @param block_number The block number on the blockchain where the claim transaction is included.
    /// @param tx_index The index of the transaction within the block.
    /// @param felt_ranges Is the range of Felt bytes of the transaction to claim for.
    /// @param is_tx Indicates whether the claim is for a transaction (true) or not (false).
    /// @param is_rx Indicates whether the claim is for a received transaction (true) or not (false).
    function submit_claim(
        uint64 chain_id,
        uint64 block_number,
        uint32 tx_index,
        FeltRange[] memory felt_ranges,
        bool is_tx,
        bool is_rx
    ) external;

    /// @dev Event emitted when a claim is submitted.
    /// @param claim_hash The hash of the claim.
    event ClaimSubmitted(bytes32 claim_hash);

    /// @notice Register prover
    /// @param nickname The nickname of the prover.
    function register_prover(string memory nickname) external;

    /// @notice Chain Price Config struct
    struct ChainPriceConfig {
        uint64 chain_id;
        uint256 price;
    }

    /// @notice Set chain price configurations
    /// @param chain_price_configs The chain price configurations.
    function set_chain_price_configs(ChainPriceConfig[] memory chain_price_configs) external;

    /// @notice Submit proof for a claim.
    /// @param claim_hash The hash of the claim.
    /// @param proof The proof data associated with the claim.
    function submit_proof(bytes32 claim_hash, bytes memory proof) external;

    /// @dev Event emitted when a proof is submitted.
    /// @param claim_hash The hash of the claim for which the proof was submitted.
    event ProofSubmitted(bytes32 claim_hash);
}
