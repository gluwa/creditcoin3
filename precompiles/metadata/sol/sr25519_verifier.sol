// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The Sr25519Verifier precompile address
address constant SR25519_VERIFIER_ADDRESS = 0x00000000000000000000000000000000000013B9;

Sr25519Verifier constant SR25519_VERIFIER_CONTRACT = Sr25519Verifier(SR25519_VERIFIER_ADDRESS);

/// @title Sr25519Verifier interface
/// @notice This interface defines the function for verifying Substrate sr25519 signatures.
interface Sr25519Verifier {
    /// @dev Verifies an sr25519 signature.
    /// @param message The message that was signed.
    /// @param signature The 64-byte sr25519 signature.
    /// @param publicKey The 32-byte sr25519 public key.
    /// @return bool true if the signature is valid, false otherwise.
    function verify(bytes calldata message, bytes calldata signature, bytes32 publicKey) external view returns (bool);
}
