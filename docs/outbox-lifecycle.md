# Outbox message authorization and recovery

Attestors cover every Outbox registered for their write-ability chain key. The Discovery
`defaultOutbox` selects a publishing default; changing it does not revoke other instances or
cancel buffered messages.

The listener scans `MessagePublished` events in bounded source-block ranges. For every candidate
it reads the chain-info Discovery address at the message's finalized Creditcoin block, then calls
that registry's `isActiveOutbox(chainKey, emitter)` at the same block. Permissionless deployments
and messages outside their registered lifecycle are excluded. Registry-address replacements,
removal cancellations, and later re-registrations are resolved from historical state, rather than
inferred from today's active list. Authority uses end-of-block state: a scheduled removal at block
N excludes messages at N; messages through N−1 remain eligible for recovery.

Result-count limits trigger smaller queries. If a single block exceeds the RPC log limit,
including because of unauthorized emitters, the listener retrieves that block's transaction
receipts and filters their logs locally. RPC outages still fail the current chunk immediately.

One durable cursor represents the fully scanned range across **all** Outboxes. It advances only
once the entire range has been authenticated and handed to the signing pipeline. Each chunk caches
Discovery/membership reads by block and emitting address. Failed or unavailable historical reads
leave the current chunk pending for retry. Reobservation uses the same historical authority and a
single-block, message-ID-filtered query, so removal cannot strand messages awaiting quorum.

The Creditcoin EVM RPC must retain historical state for the configured scan/recovery range and
support historical `eth_call`, including the chain-info precompile. Pruned state causes signing to
pause on the affected range; it never authorizes a message. On upgrade from the old single-Outbox
cursor, the scanner replays from genesis once (or the explicit `start_block` floor), since the old
cursor did not cover non-default instances. Set a known activation floor when older runtime states
do not expose the Discovery precompile; choosing a floor excludes earlier messages by operator policy. Already published votes may repeat and are deduplicated downstream. New deployments
retain the configured `start_block` behavior. Backfill can be costly because all matching event
emitters must be checked, including unauthorized deployments.

This lifecycle policy does not change finality policy or destination-key governance. The route's
destination key is captured in each indexed message so governance can reject a buffer created
under an obsolete signing domain.

## Operator rollout requirements

1. Use a Creditcoin archive endpoint for **every block in the scan and recovery range**, including
   historical chain-info and Discovery `eth_call` reads. This applies to external operators as
   well as the managed fleet. Verify representative old blocks through the actual configured
   endpoint; a current-head health check does not establish archive support. The listener logs
   this requirement at startup, and reports unavailable history while retrying the affected range.
2. Before rolling the image, set the operator-approved `writeAbility.startBlock` in each
   AttestorSet CR (`start_block` in the generated attestor configuration). Include all deployed
   routes, including Base Sepolia. Keep existing cursor files. Legacy single-Outbox cursors cannot
   prove all-Outbox coverage, so migration intentionally replays from this floor or genesis.
   Choose a floor no later than any message that needs automatic recovery. A recent devnet floor
   limits replay and repeat gossip, but explicitly excludes older automatic recovery; an old
   scanned cursor is not proof of downstream delivery. Arrange explicit reobservation for older
   pending messages, with archive access at those blocks.
3. Coordinate relayer support before enabling publications on non-default registered Outboxes.
   The current default-only relayer does not discover those publications even when attestors
   produce a quorum. Relayer multi-Outbox discovery, durable scanning, and drain/recovery are the
   remaining deployment gate tracked in
   [creditcoin3 #1373](https://github.com/gluwa/creditcoin3/issues/1373). Merging the attestor change
   does not satisfy this gate.
4. Verify end-to-end delivery from both the default and another registered Outbox, then change
   the default and verify backlog delivery. Verify pre-removal messages remain recoverable after
   removal takes effect, while publications at/after effective removal are rejected. Observe scan
   progress and historical-RPC errors throughout the rollout.
