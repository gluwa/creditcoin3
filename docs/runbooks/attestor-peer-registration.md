# Attestor networking-key registration

Attestors publish an Ed25519 networking public key on CC3. Every node reads the finalized
committee registry before starting P2P and reserves capacity for those peer identities before
receiving any votes. The registry refreshes every 15 seconds in a separate task, so RPC retries
cannot block the swarm. Boot-node addresses still provide discovery; a public key identifies a
peer without specifying its reachable address.

## What is stored and who controls it

- `Attestation.AttestorPeerKeys(chain_key, controller)` stores a 32-byte Ed25519 public key.
- `Attestation.PeerKeyOwner(chain_key, public_key)` prevents two controllers claiming the same
  networking identity on one source chain. The same controller/key may serve different chains.
- `Attestation.PeerRegistrationRequired(chain_key)` controls staged enforcement; it defaults to
  `false` for existing chains.
- `AttestorApi_attestor_peer_registry` returns the active committee, including explicit missing
  keys, and the enforcement setting from one queried block. The runtime API is version 2.

`set_attestor_peer_key(chain_key, public_key, proof_of_possession, expected_previous_key)` must be signed by the
registered **controller**, rather than the stash. The Ed25519 proof signs the shared primitive's
SCALE-encoded tuple `(domain, CC3 genesis hash, source chain key, encoded controller, public key)`.
The domain is `CC3:ATTESTOR:PEER:v1`. Copying another controller's proof or using one from another
CC3 network/source chain cannot claim its key. Both the controller transaction and networking-key
proof are required. Vote eligibility and BLS verification remain separate.

The previous-key argument provides an atomic state check. A stale first registration or rotation
cannot overwrite a newer key; the client must reread finalized state and explicitly choose the
intended transition. Retrying an already-applied identical key is an idempotent no-op.

The maps are additive: existing `Attestor` records retain their encoding and need no bulk
migration. Unregistration releases both peer-key indexes. Removed-chain cleanup clears them in
bounded batches and removes the enforcement flag. New cleanup cursor variants are appended,
preserving existing cursor encodings. Runtime `try_state` checks the two indexes and required
active keys.

## Normal operation

Use the existing attestor configuration and controller secret. On startup the binary checks its
finalized peer-key registration:

- Missing key: submit a controller-signed registration and wait for finalization.
- Matching key: continue without another registration transaction.
- Different key: stop with instructions; normal restarts cannot silently overwrite a rotation.

By default the networking key derives from the existing attestor secret exactly as before, so
existing peer IDs remain stable. An optional `--p2p-secret`, `ATTESTOR_P2P_SECRET`, or `p2p.secret`
selects a separate networking mnemonic/32-byte seed without changing the controller or BLS key.
Keep these secrets in the deployment's existing secret store.

Finalized registered keys are authoritative. Repackaging a valid BLS vote under a different
gossipsub signer cannot replace the binding or obtain a reserved slot. Unknown peers still share
the 64-connection unreserved pool, and the existing global/per-peer connection limits still apply.
Connections from boot nodes remain explicitly trusted by configuration.

## Rollout for an existing network

1. Build the runtime and matching binaries/types from this feature (3.140.0 / spec version 140,
   following release 139). Coordinate the final release version through the normal release process.
   Existing binaries enforce Attestation metadata compatibility, so coordinate the finalized
   runtime upgrade and binary rollout using the normal network upgrade procedure.
2. Leave `PeerRegistrationRequired` disabled while existing controllers publish keys. Existing
   registrations, staking records and BLS keys are retained.
3. Register each controller's networking key with the new binary. It must already be registered
   as an attestor and have funds for the native transaction fee. For example:

   ```bash
   # ATTESTOR_SECRET comes from the existing controller secret store.
   # ATTESTOR_P2P_SECRET is optional; omit it to preserve the old peer ID.
   ./attestor --config /etc/attestor/config.yaml --register-peer-key-only
   ```

   Registration-only mode uses CC3 RPC. It does not need a source-chain RPC, wait for election,
   start P2P, or start attestation production. `--chain-key` and `--cc3-url` may also supply the
   existing configuration values. It is safe to rerun when the finalized key already matches.
4. Check `attestorPeerKeys` for every active and waiting controller. Start the upgraded nodes and
   verify their networking identities and connectivity. During backfill, `p2p.reserved_peers`
   remains an optional trusted fallback for unregistered peers. A root seed with no static route
   waits in startup for finalized committee keys rather than requiring a manually maintained
   list. A strict single-member committee can bootstrap without a remote peer.
5. Through the network's existing operator/governance mechanism, submit:

   ```text
   Attestation.set_peer_registration_required(chain_key, true)
   ```

   The equivalent Polkadot JS call is
   `api.tx.attestation.setPeerRegistrationRequired(chainKey, true)`. The pallet rejects enabling
   it until active/waiting/leaving registrations and the current active set have keys. Idle
   controllers may backfill later, before their next activation.
6. Once enabled, `attest()` requires a networking key before moving a controller to `Waiting`.
   Nodes reject incomplete strict registries and discard manual/learned committee reservations;
   finalized on-chain identities provide the reservations. Boot-peer trust remains separate.
7. Remove obsolete manual `reserved_peers` lists from configuration. Keep usable boot-node
   addresses on nodes that need outbound discovery.

While enforcement is disabled, a member without a registered key still uses the legacy
vote-learning path. Complete the backfill and enforcement steps to protect every committee peer
from the first-connection bootstrap gap. A partial registry is not a completed rollout.

## Rotate a networking key

1. Prepare a new independent networking secret and plan the brief connectivity transition.
2. Stop the old instance and use the controller secret plus the new `ATTESTOR_P2P_SECRET` to run:

   ```bash
   ./attestor --config /etc/attestor/config.yaml \
     --register-peer-key-only --rotate-peer-key
   ```

3. After finalization, start the attestor normally with the new networking secret. Omit
   `--rotate-peer-key` on ordinary starts so an old deployment cannot silently revert the key.
4. Other nodes pick up the finalized change on their next registry refresh, revoke the old
   reservation and disconnect the obsolete identity. An instance observing that its own
   finalized key changed stops with a configuration error. Update any boot multiaddrs that
   explicitly contained the old Peer ID; identity registration does not rewrite addresses.

The controller and BLS identities remain unchanged. The old networking identity loses committee
reservation privileges; unknown connections continue to use the bounded unreserved policy.

## Failure and recovery behavior

| Situation | Behavior |
|---|---|
| Missing/insufficient controller registration or funds | Registration fails clearly; no random networking identity is substituted |
| A different key is already registered | Startup refuses automatic replacement; operator chooses the matching key or explicit rotation |
| Runtime registry has duplicate identities or missing required keys | Refuse the invalid snapshot |
| RPC outage after startup | Preserve the last qualified finalized registry while the separate task retries; existing health/watchdog behavior still applies |
| Reconnected RPC is behind the retained finalized height | Ignore the older view; it cannot restore a revoked reservation |
| Conflicting registry at the same finalized height or a changed genesis | Stop on inconsistent chain identity/state |
| Member chilled, removed or rotated | Finalized refresh rebuilds committee reservations and disconnects peers that lost reserved status |
| Emergency enforcement rollback | Operator may set the flag to `false`; stored registrations stay authoritative and the configured legacy fallback becomes available again |

The 15-second refresh cadence is eventual reconciliation after finalization, rather than an
instantaneous cross-network cutover. Keep RPC access healthy and coordinate rotations accordingly.

## Weight and validation notes

The new calls use conservative envelopes based on the existing measured BLS-registration and
full-election CPU budgets, with additional DB work and storage proof allowances. These are
deliberately conservative bounds, not newly claimed benchmark measurements. Dedicated benchmarks
cover key rotation and policy activation across the registered set so the regular benchmarking
process can replace the envelopes with direct measurements.

Regression coverage includes proof replay across controllers/networks/chains, uniqueness,
rotation and cleanup, staged enforcement, full-pool first-connection admission, registered peer
bindings resisting copied votes, reservation revocation, stale registry rejection, and libp2p /
runtime Ed25519 proof compatibility. Regenerated Rust and CLI metadata accompany the runtime API.
