# Proof Of Concept

## Description

In this document we will explain the proof of concept for query creation and proving. This is a full blown example from running the execution chain, source chain, prover, and attestation network.

Components in this proof of concept:

- Execution chain: Ccnext chain
- Source chain: Ethereum compatible chain
- Prover: Prover module that will deploy a Prover contract on ccnext chain where we can create and prove queries.
- Attestation network: Attestor module that will connect to the Ethereum compatible chain and attest to blocks on ccnext chain. In this PoC orchestrated by the `attestor_zombienet`.
- Query client: Client that will create queries and send them to the prover.

## Useful links

### Devnet deployment

- Ccnext: [https://rpc.ccnext-devnet.creditcoin.network](https://rpc.ccnext-devnet.creditcoin.network) for http, websocket: [wss://rpc.ccnext-devnet.creditcoin.network](wss://rpc.ccnext-devnet.creditcoin.network)
- Ethereum (anvil): [https://anvil.ccnext-devnet.creditcoin.network](https://anvil.ccnext-devnet.creditcoin.network)
- Gluwa public prover contract address: `0x21cb3940e6ba5284e1750f1109131a8e8062b9f1`
- PolkadotJS explorer link: [https://polkadot.js.org/apps/?rpc=wss%3A%2F%2Frpc.ccnext-devnet.creditcoin.network%2Fws#/explorer](https://polkadot.js.org/apps/?rpc=wss%3A%2F%2Frpc.ccnext-devnet.creditcoin.network%2Fws#/explorer)
- Anvil explorer: [https://explorer.ccnext-devnet.creditcoin.network/](https://explorer.ccnext-devnet.creditcoin.network/)

## Pre-requisites

- Rust
- Docker

## 1. Build the project

Build the workspace in release mode

```sh
cargo b --features=fast-runtime --release`
```

## 2. Start solo chain

```sh
./target/release/creditcoin3-node --dev --tmp
```

Once started navigate to [polkadot explorer](https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944#/explorer) to see the blockchain explorer.

## 3. Start anvil

This will run an ethereum local network that will be used to simulate transactions.

This is part of foundry, see [installation](https://book.getfoundry.sh/getting-started/installation). See anvil [reference](https://book.getfoundry.sh/reference/anvil/).

```sh
anvil --block-time 6
```

## 4. Start attestor zombienet

First configure to connect to local chain, see `creditcoin3-next/attestor_zombienet/config.yaml`

set:

```toml
single_node: true
```

start zombienet:

```bash
cd attestor_zombienet
../target/release/attestor_zombienet --cc3-key "//Alice"
```

Now check if attestations are coming through on the polkadot explorer. There should be events like: `AttestationSubmitted`.

## 6. Start prover

This is prover module that will deploy a Prover contract on ccnext chain where we can create and prove queries.

First go to the prover folder

```sh
cd prover
```

Start Docker compose:

```sh
docker compose up -d
```

Create artifacts dir:

```sh
mkdir artifacts
```

Install and run cairo environment:

First time on machine: 
```sh
python3.10 -m venv ~/cairo_venv
```
Every prover run:
```sh
source ~/cairo_venv/bin/activate
pip install -r requirements.txt
```

Now start the prover:

```sh
./target/release/prover \
  --cc3-key "//Alice" \
  --eth-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133"
```

You should see something like

```sh
2024-08-16T12:26:51.156791Z  INFO prover::postgres::db: Running databse migrations...
2024-08-16T12:26:51.218675Z  INFO prover: Created attestations cache
2024-08-16T12:26:51.223392Z  INFO prover::contract: Deploying Gluwa Public Prover contract
2024-08-16T12:27:00.253137Z  INFO prover::contract: Creditcoin Public Prover contract address(0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3) on chain 42
2024-08-16T12:27:00.253170Z  INFO prover: Deployed prover contract
2024-08-16T12:27:00.305659Z  INFO prover: Building historical cache for chain with id: 42
2024-08-16T12:27:00.305690Z  INFO prover::attestation_cache: Building historical cache for chain
```

Once started it will log the prover contract address. Copy this address and use it in the next module. In this example it is `0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3`.

## 7. Make a transfer

This will transfer some amount from `Alice` to some other random account.

```sh
cd attestor/scripts
node transfer.js
```

Save the ouput to create a Query later.

## 8. Query cli

Create a query, first check on the anvil logs for a transaction in a block. Currently it's only possible to create queries for blocks that are attested to,
so either check the prover logs for attestations that are being cached or check the chain.

See transfer section for how to make a transfer. Take note of the block number and transaction hash.

to create a query, run the query cli:

```sh
cd query-cli
cargo run -- \
  --cc3-rpc-url http://localhost:9944 \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --prover-contract-address 0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3 \
  --infura-api-key "somekey" \
  --eth-rpc-url http://localhost:8545
```

> You can leave the infura-api-key value like the example one if you are using a local chain.

Select:

- Local chain
- block number: 348
- transaction hash: 0x584ee77611d71f6bd4c1459f08da01b80208ab04a4f3c67c26207b02765a1cd1
- all data

Now the prover should run the query and prove it. The result is submitted back to the cli and eventually it exits.

## 9. Resetting After Tests
Whenever you start up a new chain as in step 2 there is an additional cleanup consideration.

1. This file must be deleted with each restart, `artifacts/chain_deployment_artifacts.json`
2. The prover DB must be cleaned of all entries so that it doesn't retain information from past tests.

There are many ways to clean your db, but one is to connect to your local db using a management GUI such as DBeaver.
You can then run DELETE queries on the various tables.

Failing to clean the DB can result in multiple attestations, blocks, or checkpoints being present at each block height.

Some of those will have the wrong digests, as they were saved from past chains.
This can cause mismatches when proving claims.

## 10. Creating a query against devnet

To create a query against the devnet, you first need to run a transfer.

```sh
cd attestor/scripts
node transfer.js --devnet
```

This will output a block number and transaction hash. Use these values to create a query.

```sh
cd query-cli
cargo run -- \
  --cc3-rpc-url https://rpc.ccnext-devnet.creditcoin.network \
  --eth-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --contract-address 0x21cb3940e6ba5284e1750f1109131a8e8062b9f1 \
  --infura-api-key "somekey" \
  --eth-rpc-url https://anvil.ccnext-devnet.creditcoin.network
```

Now you can wait for the prover to finish proving the query.

## 11. Running the Prover in Light Mode

When run in light mode, the prover only schedules and provides inputs for proving jobs.
The actual proving work is delegated to an Azure data pipeline mediated by a 
prover-be-api server. That server will soon be hosted on Kubernetes for devnet, but for
now you need to build and launch your own with docker.

To set up the prover-be-api server, clone the code base found [here](https://dev.azure.com/gluwa/Gluwa/_git/CCNext.StoneProver.BE). Then follow the steps in its readme.

Note the exposed socket address of your prover-be-api server. In place of "http:// localhost:55644" below, use the socket exposed by your target prover-be-api instance. 

In light mode you must also provied a UUID api key for requests sent to the prover backend server with the argument `--be-api-key`. Api keys are managed by the prover BE server administrator. So you need to ask them for a key. If you are launching your own BE server, then you need to look up or create a valid api key for your server.

```sh
./target/release/prover \
--cc3-key "//Alice" \
--cc3-evm-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133" \
--prover-be-socket-addr "http:// localhost:55644" \
--be-api-key "f40677cb-8aa5-4a8e-bb99-2933b12b473c" \
--postgres-uri "postgres://prover:prover@127.0.0.1:5432/attestations"
```

When set up correctly, the light prover will send proving requests to the prover-be-api server. Then in a few minutes the server will respond with an output proof file.

When sending queries to BE instances not hosted locally, use an address prepended with "https://". EX: --prover-be-socket-addr "https:// 122.0.38.55:55644"