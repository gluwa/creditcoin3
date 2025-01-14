# Query CLI

This simply CLI tool allow you to create a "Query" Object that can be used to submit to a public prover and get the proof result.

## Prerequisites

You need to run following components first:

- Ccnext network (dev or whatever)
- Prover binary (see [prover](../prover/README.md) for more details)

## Installation

```sh
cargo build --release
```

## Usage

For all available options, run:

```sh
../target/release/query-cli --help
```

## Example

```sh
../target/release/query-cli \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --infura-api-key "somevalue" \
  --prover-contract-address 0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3
```

## Default example

There is a flag that will enable you to submit a "default" query, it's a query made on sepolia about this transaction:
<https://sepolia.etherscan.io/tx/0xa519add3d602460c2b30c7ff4b1215fd705f049bb87260c2a2fc8fe2c3ccce9a>.

If you pass the default flag you wont get the prompt and you can test the query submission faster.

```sh
../target/release/query-cli \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --prover-contract-address 0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3 \
  --default
```
