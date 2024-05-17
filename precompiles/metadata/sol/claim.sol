// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The Deposit precompile address
address constant CLAIM_CONTRACT_ADDRESS = 0x0000000000000000000000000000000000003049;

ClaimContract constant CLAIM_CONTRACT_ADRRESS = ClaimContract(CLAIM_CONTRACT_ADDRESS);

/// @title ClaimContract interface
interface ClaimContract {
    /// @dev Event emitted when a claim is submitted.
    /// @param claim_id The Id of the claim
    event ClaimSubmitted(uint64 claim_id);

    function submit_claim(uint64 chain_id, uint64 block_number, uint8 tx_index, address from, address to, bool is_tx, bool is_rx) external;
}
