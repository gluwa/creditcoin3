// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title MockWriteAbilityEmitter
/// @notice Minimal event emitter used by the cc3-indexer integration tests to exercise the
///         write-ability EVM handlers (`handleOutboxCreated`, `handleMessagePublished`,
///         `handleMessageAcknowledged`) without standing up the whole fee stack (AttestorVault,
///         ATTEST token, validator, quoter, CREATE2 OutboxFactory).
///
///         This is sound because the indexer discovers Outboxes by *topic*, chain-wide, rather than
///         from a known factory address — see `outboxDiscoveryDatasource` in
///         `cc3-indexer/datasources.ts` and the "P2-10 factory correspondence" note in
///         `src/mappings/evmHandlers.ts`. A contract that is not a registered `OutboxFactory` is
///         exactly the *unauthenticated* discovery path, so these tests cover it as written.
///
/// @dev The three event signatures below MUST stay byte-identical to the canonical definitions in
///      the `usc-contracts` repo (`contracts/write-ability/Outbox.sol` and
///      `deployer/OutboxFactory.sol`) and to the ABIs the indexer matches against
///      (`cc3-indexer/abis/outbox_factory.json`, `cc3-indexer/abis/outbox.json`). A drift in
///      argument types or `indexed` flags changes the topic hash, and the handlers would silently
///      stop firing.
contract MockWriteAbilityEmitter {
    /// Mirrors `OutboxFactory.OutboxCreated` — topic
    /// `OutboxCreated(address,uint32,address,address,string)`.
    event OutboxCreated(
        address indexed outbox,
        uint32 indexed chainKey,
        address indexed owner,
        address validator,
        string version
    );

    /// Mirrors `Outbox.MessagePublished` — topic `MessagePublished(bytes32,bytes32,bool,bytes)`.
    /// `emitterAddress` is a bytes32 holding the 20-byte EVM address in the high bytes.
    event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload);

    /// Mirrors `Outbox.MessageAcknowledged` — topic `MessageAcknowledged(bytes32)`.
    event MessageAcknowledged(bytes32 indexed messageId);

    /// @notice Announce this contract as an Outbox for `chainKey`.
    /// @dev `outbox` is deliberately `address(this)`: the indexer registers a dynamic datasource for
    ///      the announced address, so pointing it back here lets this same contract then emit the
    ///      `MessagePublished` / `MessageAcknowledged` events that datasource listens for.
    function emitOutboxCreated(uint32 chainKey, address validator, string calldata version) external {
        emit OutboxCreated(address(this), chainKey, msg.sender, validator, version);
    }

    function emitMessagePublished(
        bytes32 messageId,
        bytes32 emitterAddress,
        bool requiresAck,
        bytes calldata payload
    ) external {
        emit MessagePublished(messageId, emitterAddress, requiresAck, payload);
    }

    function emitMessageAcknowledged(bytes32 messageId) external {
        emit MessageAcknowledged(messageId);
    }
}
