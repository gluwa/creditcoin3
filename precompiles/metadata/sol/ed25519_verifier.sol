// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The Ed25519Verifier precompile address
address constant ED25519_VERIFIER_ADDRESS = 0x00000000000000000000000000000000000013bA;

Ed25519Verifier constant ED25519_VERIFIER_CONTRACT = Ed25519Verifier(ED25519_VERIFIER_ADDRESS);

/// @title Ed25519Verifier interface
/// @notice This interface defines the function for verifying ed25519 signatures.
interface Ed25519Verifier {
    /// @dev Verifies an ed25519 signature.
    /// @param message The message that was signed.
    /// @param signature The 64-byte ed25519 signature.
    /// @param publicKey The 32-byte ed25519 public key.
    /// @return bool true if the signature is valid, false otherwise.
    function verify(bytes calldata message, bytes calldata signature, bytes32 publicKey) external view returns (bool);
}
