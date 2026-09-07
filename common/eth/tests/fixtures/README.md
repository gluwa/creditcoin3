# OP-Stack fixtures

- `base_sepolia_46388021_*`: the original Base Sepolia block and receipt fixtures from PR #1319.
- `op_mainnet_110000000_*`: a pre-Canyon OP mainnet block and matching receipts from the official
  OP RPC. Publicnode omits the deposit's transaction `nonce` for this same block, which motivated
  the missing-metadata regression. Tests verify both header roots before using any leaf.

The deposit common nonce is the receipt-root-authenticated `depositNonce` from Canyon onward.
Before Canyon the leaf stores zero (unavailable): RPC nonce metadata is not committed by either
header root and must not affect the attestation root. This is a leaf-format convention, not a
claim that the historical execution nonce was zero. See the [OP deposit specification](https://specs.optimism.io/protocol/deposits.html).
