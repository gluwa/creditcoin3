// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The SignatureVerifier precompile address
address constant SIGNATURE_VERIFIER_ADDRESS = 0x00000000000000000000000000000000000013B9;

SignatureVerifier constant SIGNATURE_VERIFIER_CONTRACT = SignatureVerifier(SIGNATURE_VERIFIER_ADDRESS);

/// @title SignatureVerifier interface
/// @notice This interface defines the function for verifying Substrate sr25519 signatures.
interface SignatureVerifier {
    /// @dev Verifies an sr25519 signature.
    /// @param message The message that was signed.
    /// @param signature The 64-byte sr25519 signature.
    /// @param publicKey The 32-byte sr25519 public key.
    /// @return bool true if the signature is valid, false otherwise.
    function verify(bytes calldata message, bytes calldata signature, bytes32 publicKey) external view returns (bool);
}
