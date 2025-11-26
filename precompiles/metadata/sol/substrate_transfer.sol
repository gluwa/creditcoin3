// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The Deposit precompile address
address constant SUBSTRATE_TRANSFER_ADDRESS = 0x0000000000000000000000000000000000000Fd1;

SubstrateTransfer constant SUBSTRATE_TRANSFER_ADRRESS = SubstrateTransfer(SUBSTRATE_TRANSFER_ADDRESS);

/// @title SubstrateTransfer interface
interface SubstrateTransfer {
    /// @dev Event emitted when a transfer has been performed.
    /// @param destination The Substrate address receiving the tokens.
    /// @param amount The amount of tokens transferred.
    event Transfer(bytes32 destination, uint256 amount);

    /// @dev Function to transfer tokens to a Substrate address.
    /// @param destination The Substrate address receiving the tokens.
    /// @param amount The amount of tokens to transfer.
    function transfer_substrate(bytes32 destination, uint256 amount) external;
}
