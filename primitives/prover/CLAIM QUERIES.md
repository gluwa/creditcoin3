# Claim queries - current iteration

This text describes implementation details of a protocol for claim query submission a to prover and delivering results back from the prover to claimer.

## Claim query

A claim query is an array of tags that an end user enumerates to specify fields of interest of a source chain block data to be fetched by the prover and delivered back to the claimer.
The claim query along with identification data (tx/rx, block number, index) constitutes a claim.

## Assumptions

- prover and claimer are mutually trustfullnessless
- claim query submitted by a user may contain an arbitrary number (bounded to a defined max value) of tags in arbitrary order
- claim queries refer to rlp-encoded entities only
-
## Data structures

For any recognizable type of transaction or receipt an enum of corresponding tags is to be defined and implemented by extending the `ClaimQueryField` trait.
The enum must implement a casting from `usize` to exactly correspond to the data structure layout the tags refer to.
The enum `SampleBy` specifies a mode of sampling for a specific field of the data structure the tag corresponds to.
The sampling modes are:
```rust
pub enum SampleBy {
    // sample a single value
    Value,
    // sample a byte range [a..b). If None, entire sampling range is implied
    Range(Option<Range<usize>>),
    // sample an array element by index. If None, entire array is implied
    Index(Option<usize>),
}
```
For instance, in order to sample a single value like `ChainId` or `Signature` `SampleBy::Value` is to be used, while `SampleBy::Range(Some(a..b))` may be used for sampling a singe continuous range within a byte array.
`SampleBy::Index(Some(i))` can be used for sampling a single value from a typed array by index, like `Logs` or `AccessList`.
`SampleBy::Range(None)` and `SampleBy::Index(None)` refer to sampling of an entire range of values.

## Protocol

### Claimer side

- fetch an entity of interest at specified index of a given block
- extract an rlp-encoded payload from the entity
- wrap it with an Rlp object instance
- compose a claim query (array of tags)
- run a compaction and ordering procedure on the claim query in order to transform it to a strictly ascending order, non-overlapping byte offset ranges.
- transform byte offset ranges to Starknet field element offset ranges.
- pack the claim id and query into a serializable object and send it to the prover
- recieve stone proof and public memory from the prover.
- verify stone proof.
- verify continuity
- assert the claim id in the proof's public memory to match the original claim id.
- hash the query fields and assert them to match the query hash in the proof's public memory.
- assert each value in the proof's public memory to match the local raw rlp data at corresponding byte offset range.

### Prover side
- receive claim from the claimer
- repeat the compaction and ordering procedure on the claim query exactly as on the claimer side (we can't trust the claimer indeed ran it).
- fetch corresponding block data and localize the entity of interest by index if possible, otherwise extract out-of-bounds witness for this block.
- generate a Merkle path for data or out-of-bounds witness.
- form the stark program private input containing block number, raw data field elements, merkle path, attestation slice and claim query field elements.
- run the stark program.
- generate stone proof with public memory containing attestation data, claim query hash and data field elements, sampled at offsets specified by the query, or the out-of-bounds witness
- send the stone proof back to the claimer

## Claim index out-of-bounds case - current state

This is a situation where the claimer by mistake or maliciously submits a claim concerning inexisting data, i.e. it's index exceeds the entity count in the block.

The task of the prover in this situation is to provide a committed proof of the fact the provided query is "out-of-bounds".
As a Merkle path can only verify statements regarding information inclusion, we need a special treatment to provide committment to absence of queried data.

### First approach for exclusion proving - compatible with Merlkle Mountian Range as underlyinng commitment data structure (laid off for now)

Involves inclusion of a hash of an rlp-encoded entity count as a last leaf in the merkle tree (`ClaimOutOfBoundsWitness` instance).

As it's an rlp-encoded single integer value, there is no risk of clashing with existing transaction/receipt which are rlp-encoded as lists rather than single value and rlp encoding is deterministic.

The claimer will check the matching of the claimed and received indices and, if the received index is smaller than the claimed one, it will assert the inclusion of a "out-of-bounds witness".

### Second approach for exclusion proving - supported by a standalone Merkle Tree only as underlyinng commitment data structure - adopted

No additional data is committed to, the algorithm relies on a detereministic MT height.
The MT leaves are padded with "null hashes" and in out-of-bounds case the merkle path to the first null leaf is verified, the constraint leaf == "null leaf" is applied and the "null leaf" index is computed from the merkle path height.
This index is output as out-of-bounds witness.
In case when the Merkle tree is full and no "null leaf" is available, the last leaf Merkle path is used and the constraint on it's index to be the last index in the tree is applied (computable from the Merkle path).

## Claim query ordering procedure

Prover relies on the fact a honest claimer submit query offset range field elements in ascending order, so when the data field elements are output in the same order, the claimer can parse them using it's own copy of query offset ranges.

To be able to define partial ordering on ranges `[r0, r1, ... rn] `, their mutual intersection sets must be empty, so a prior compaction (merging) procedure is necessary.

The claimer runs the compaction and ordering routine before making a claim.

However the prover can't trust the claim query origin, so it is to run the same routine to prevent a performance degradation attack on its stark program.

The compaction procedure runs with `O(n^3)` complexity in worst case where `n` is number of input ranges.

Maximum query size is limited to prevent stack overflow of the compaction procedure when a huge query is submitted.
