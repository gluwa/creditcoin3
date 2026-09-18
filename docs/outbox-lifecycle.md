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

One durable cursor represents the fully scanned range across **all** Outboxes. It advances only
once the entire range has been authenticated and handed to the signing pipeline. Each chunk caches
Discovery/membership reads by block and emitting address. Failed or unavailable historical reads
leave the current chunk pending for retry. Reobservation uses the same historical authority and a
single-block, message-ID-filtered query, so removal cannot strand messages awaiting quorum.

The Creditcoin EVM RPC must retain historical state for the configured scan/recovery range and
support historical `eth_call`, including the chain-info precompile. Pruned state causes signing to
pause on the affected range; it never authorizes a message. On upgrade from the old single-Outbox
cursor, the scanner replays from genesis once, since the old cursor did not cover non-default
instances. Already published votes may repeat and are deduplicated downstream. New deployments
retain the configured `start_block` behavior. Backfill can be costly because all matching event
emitters must be checked, including unauthorized deployments.

This lifecycle policy does not change finality policy or destination-key governance. The route's
destination key is captured in each indexed message so governance can reject a buffer created
under an obsolete signing domain.
