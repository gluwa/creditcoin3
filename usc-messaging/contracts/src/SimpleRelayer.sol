// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Simple relayer contract for PoC. Validates signed quotes and forwards payment to payee.
contract SimpleRelayer {
    /// @notice Mirrors the off-chain SignedQuote type produced by the quoter service.
    struct SignedQuote {
        bytes32 messageId;
        uint256 relayPrice;
        uint256 acknowledgmentPrice;
        address payeeAddress;
        address paymentToken;
        uint256 expiry;
        bytes signature; // 65-byte ECDSA signature (r ++ s ++ v)
    }

    /// @notice Registered quoter addresses whose signatures are accepted.
    mapping(address => bool) public registeredQuoters;

    event FeeCollected(address indexed from, uint256 amount);
    event MessagePaid(bytes32 indexed messageId);
    event QuoterRegistered(address indexed quoter);
    event QuoterRemoved(address indexed quoter);

    error InvalidSignatureLength();
    error QuoteExpired();
    error UnregisteredQuoter(address recovered);
    error InsufficientPayment(uint256 required, uint256 provided);
    error TransferFailed();

    constructor() {}

    // ─── Quoter management ──────────────────────────────────────────────

    /// @notice Register a quoter address so its signed quotes are accepted.
    function registerQuoter(address quoter) external {
        registeredQuoters[quoter] = true;
        emit QuoterRegistered(quoter);
    }

    /// @notice Remove a quoter address.
    function removeQuoter(address quoter) external {
        registeredQuoters[quoter] = false;
        emit QuoterRemoved(quoter);
    }

    // ─── Quote validation ───────────────────────────────────────────────

    /// @notice Validates a signed quote and collects the fee.
    /// @dev Recalculates the hash from the quote fields using the same encoding as the
    ///      off-chain quoter (solidityPackedKeccak256), recovers the signer via ecrecover,
    ///      and checks that the signer is a registered quoter.
    function validateAndCollectFee(SignedQuote calldata quote) external payable {
        // 1. Check expiry
        if (block.timestamp > quote.expiry) revert QuoteExpired();

        // 2. Reconstruct the hash (must match encodeQuoteForSigning in quote.ts)
        bytes32 hash = keccak256(
            abi.encodePacked(
                quote.messageId,
                quote.relayPrice,
                quote.acknowledgmentPrice,
                quote.payeeAddress,
                quote.paymentToken,
                quote.expiry
            )
        );

        // 3. Recover signer from the signature
        address signer = _recoverSigner(hash, quote.signature);

        // 4. Verify the signer is a registered quoter
        if (!registeredQuoters[signer]) revert UnregisteredQuoter(signer);

        // 5. Verify payment covers the quoted total
        uint256 totalPrice = quote.relayPrice + quote.acknowledgmentPrice;
        if (msg.value < totalPrice) {
            revert InsufficientPayment(totalPrice, msg.value);
        }

        // 6. Forward payment to the payee specified in the quote
        if (msg.value > 0) {
            (bool ok,) = quote.payeeAddress.call{value: msg.value}("");
            if (!ok) revert TransferFailed();
            emit FeeCollected(msg.sender, msg.value);
        }

        emit MessagePaid(quote.messageId);
    }

    // ─── Internal helpers ───────────────────────────────────────────────

    /// @dev Recovers the signer address from a raw hash and a 65-byte signature.
    ///      The quoter signs the raw hash (no "\x19Ethereum Signed Message" prefix).
    function _recoverSigner(bytes32 hash, bytes calldata sig) internal pure returns (address) {
        if (sig.length != 65) revert InvalidSignatureLength();

        bytes32 r;
        bytes32 s;
        uint8 v;

        // solhint-disable-next-line no-inline-assembly
        assembly {
            r := calldataload(sig.offset)
            s := calldataload(add(sig.offset, 32))
            v := byte(0, calldataload(add(sig.offset, 64)))
        }

        // Normalise v: ethers.js serialized signatures use 27/28
        if (v < 27) v += 27;

        return ecrecover(hash, v, r, s);
    }
}
