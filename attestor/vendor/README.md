# Vendored crates

## libp2p-gossipsub 0.47.0

A local copy of upstream `libp2p-gossipsub` 0.47.0 with the backoff fixes from
rust-libp2p gossipsub 0.49.3 and 0.49.4 backported. The root `Cargo.toml` swaps
it in through `[patch.crates-io]`.

Why a copy instead of an upgrade: those fixes ship in gossipsub 0.49.x, which
needs a newer libp2p than 0.54, and `sc-network` (polkadot-sdk) pins the
workspace to libp2p 0.54.

Keep it as close to upstream as possible: it is excluded from the workspace,
taplo, MegaLinter and the repo checks, so upstream changes can be diffed in
cleanly.

Remove it, together with the `[patch.crates-io]` entry and the workspace
`exclude`, once polkadot-sdk moves the workspace to a libp2p release that can
take gossipsub 0.49.4 or newer.
