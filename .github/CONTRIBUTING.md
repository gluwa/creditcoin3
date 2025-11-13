## Before we get started

In this document we will explain how to run `cc3-next` locally, set up query creation as well as 
proving. This is a fully featured example on how to run your own local execution chain, source 
chain, prover, and attestation network.

## Definitions

First, some definitions:

- **Execution chain**: Decentralized oracle (`cc3-next`) responsible for aggregating state proofs 
  from a _source chain_. 
- **Source chain**: Chain emitting state which is synchronized and proven to the _execution chain_. 
  For demonstration purposes we will be using Ethererum as the source chain.
- **Prover**: a node responsible for deploying a Prover contract to the `cc3-next` _execution 
  chain_. We can then query this contract to create and prove user queries.
- **Attestation network**: Decentralized network of nodes responsible for aggregating information
  about the state of a _source chain_: attestors bridge information about that chain's state to the 
  _execution chain_.
- **Query client**: a local client which requests proofs from the _prover_. In a real setup, a 
  client could be an end user querying the _execution chain_ for information about the state of a 
  _source chain_. That information is proven using zk so no trust assumptions are made.

## External dependencies

> [!TIP]
> The project also provides a `flake.nix` you can use to run any of the commands in this document.
> It will handle all external dependencies for you, keeping your dev environment clean of any clutter
> as well as helping you out by automating certain error-prone steps in env setup and teardown. Just 
> keep in mind you will have to **[enable flakes]** for this to work. Note that docker still has to 
> be installed and set up manually or the docker service socket will not be available to run!
>
> If you are just looking to spawn a dev environment to run some arbitrary commands, you can also do
> so by running:
> 
> ```bash
> nix develop
> ```

- rust
- python3.10
- openssl
- pkg-config
- protobuf
- clang
- jq
- foundry
- nodejs
- docker
- docker compose

## Environment setup

> [!IMPORTANT]
> If you are using `nix`, you can skip this step.

A lot of the commands below require the _proving_ and _verifying_ scripts inside of `cairo/` to be
made available in `$PATH`, alongside with some extra dependencies to run them. **Remember to run the
following before any of the commands below**:

> [!CAUTION]
> `cc3-next` has a hard requirement on `python3.10`. **Do not use any other version of python**, it
> very likely won't work.

```bash
# Make prover/verifier available in the PATH
PROVER_PATH=$(readlink -f ./cairo/stone-prover)
SCRIPTS_PATH=$(readlink -f ./cairo/scripts)
VERIFIER_PATH=$(readlink -f ./cairo/stone-verifier)
export PATH="$PATH:$PROVER_PATH:$SCRIPTS_PATH:$VERIFIER_PATH"

# Setup Python environment
python3.10 -m venv ~/cairo_venv
source ~/cairo_venv/bin/activate
pip install -r prover/requirements.txt
```

## Resetting After Tests

> [!IMPORTANT]
> If you are using `nix`, you can skip this step.

Whenever you start up a new chain there are some additional cleanup considerations.

1. `artifacts/chain_deployment_artifacts.json` must be deleted with each restart.
2. The prover DB must be cleaned of all entries so that it doesn't retain information from past 
   tests.

There are many ways to clean your db, but the simplest one is probably just to destroy the previous
prover postgresql instance by running:

```bash
docker compose -f ./prover/docker-compose.yaml down
```

Alternatively, if you need more fine-grained control or if you are not running a dockerized version
of the prover db, you can connect to your local db using a management GUI such as DBeaver. From 
there, you can run `DELETE` queries on the various tables.

> [!CAUTION]
> Failing to clean the DB can result in multiple attestations, blocks, or checkpoints being present
> at each block height. Some of these blocks  will have the wrong digests, as they were saved from
> past chains. This can cause mismatches when proving claims.

## 0. Build the project

> [!IMPORTANT]
> If you are using `nix`, you can skip this step.

Before anything else, start by building the workspace in release mode.

```sh
cargo build --features=fast-runtime --release`
```

## 1. Start local execution chain

> [!CAUTION]
> Have you read the section on [environment setup](#environment-setup)?

Start by running your own solo copy of the cc3-next chain.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#node
> ```
> If you get any errors, remember to **[enable flakes]**.

```bash
./target/release/creditcoin3-node --dev --tmp
```

Once that is done navigate to [polkadot js] to see the blockchain explorer. You should see it 
connect to your local chain and display the current blocks being produced.

## 2. Start anvil

> [!CAUTION]
> Have you read the section on [environment setup](#environment-setup)?

[`anvil`] is a cli tool which allows you to run an ethereum local network to simulate _source chain_
transactions. Anvil is part of foundry, see [installation instructions].

Once anvil is installed on your system, start it:

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#anvil
> ```
> If you get any errors, remember to **[enable flakes]**.

```sh
anvil --block-time 6
```

## 3. Start attestor zombienet

Now that we have a local _execution chain_ and _source chain_ set up, let's get our _attestors_ up 
and running.

First, create `creditcoin3-next/attestor_zombienet/config.yaml` if it is missing. Then set:
```toml
single_node: true
```

Now you can start zombienet:

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#zombienet
> ```
> If you get any errors, remember to **[enable flakes]**.

```bash
cd attestor_zombienet
../target/release/attestor_zombienet --cc3-key "//Alice"
```

Now check if attestations are coming through on the polkadot explorer. There should be events like: 
`AttestationSubmitted` visible on the right panel of the block explorer.

## 4. Make a transfer

We need some data on our _source chain_ for our _attestor zombienet_ to send over to our _execution 
chain_. We can do this by sending a transaction to our local `anvil` chain.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#transfer
> ```
> If you get any errors, remember to **[enable flakes]**.

To send a transaction, run:

```bash
cd attestor/scripts
node Transfer.js

```

Copy the resulting **block number** and **transaction hash**. You will need them later.

## 5. Start the prover

> [!CAUTION]
> Have you read the section on [environment setup](#environment-setup)?

Great! Our _source chain_ and _execution chain_ are up and running, and state is being relayed from
one to the other by our _attestor zombienet_. We can now run the _prover_ module which will 
deploy a _prover contract_ to `cc3-next` so we can create and prove queries.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#prover
> ```
> If you get any errors, remember to **[enable flakes]**.

First, we need to spawn a postgresql database for the prover to use:

```bash
docker compose -f ./prover/docker-compose.yaml down
docker compose -f ./prover/docker-compose.yaml up -d
```

Then, create an `artifacts` dir:

```bash
mkdir artifacts && rm -rf artifacts/chain_deployment_artifacts.json
```

Now start the prover, keep in mind the database ports defined in (prover/docker-compose.yaml):
// secretlint-disable

```sh
./target/release/prover \
  --cc3-key "//Alice" \
  --cc3-evm-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133" \
  --postgres-uri "postgres://prover:prover@127.0.0.1:5433/attestations" \
  --name "devprover"
```

// secretlint-enable

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

Once started it will log the prover contract address.

## 6. Query cli

> [!CAUTION]
> Have you read the section on [environment setup](#environment-setup)?

This is the point up to which everything has been leading up: proving on our _execution chain_ that 
state has been made available on our _source chain_: **a trustless, decentralized oracle**.

Before we continue, take a look at the output in your local prover. You will need to wait for 
something like this to appear:

```bash
INFO 📝 Received a new attestation: chain: 2, blocknumber: 20
```

Where `blocknumber` should be _greater_ than the block number displayed after you made a transfer in
[step 4]. Make sure this is the case. Otherwise, your query will fail. This is
due to the fact that **currently it is only possible to create queries for blocks that have already 
been attested to**.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#query
> ```
> If you get any errors, remember to **[enable flakes]**.

To create a query, run the query cli:

```sh
./target/release/query-cli \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --prover-contract-address 0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3 \
```

> [!NOTE]
> You will need to provide an Infura API key if you want to connect to a remote Ethereum or Sepolia
> node. The cli will prompt you for an api key.

Select:

- **Network**: Local
- **Network URL**: Leave as is
- **Block height**: The block number you save from [step 4].
- **Transaction hash**: The transaction hash you saved from [step 4].
- **Data to represent**: Native token transfer data

You should see the following appear in the prover's output (it might take some time for the full
output to become available, proving is quite slow):

```bash
INFO 📝 Received query [...], checking for readiness...
INFO ✅ Query [...] is ready for immediate processing.
INFO 🔄 Processing unprocessed query
INFO 📝 Prepared query for proving with id
INFO 📝 Submitting proof for query
INFO 🏁 Proof verified successfully for query
INFO 🛰️ Proof verification event received for query
```

This will:

- Receive the query you just sent.
- Generate a merkle proof of its existence based on the data attested to by the _attestor zombienet_.
- Send the proof back to the _execution chain_ for inclusion in a block via the _prover contract_.
- Verify the proof on the _execution chain_ via the _prover contract_.

While this is happening, the _query cli_ will keep monitoring the state of the _execution chain_ and
exit as soon as its query has been verified.

# Advanced

This section contains extra information on more advance topics which are currently harder to setup
locally. More documentation is probably needed to make this clearer.

## Creating a query against devnet

To create a query against the devnet, you first need to run a transfer.

```sh
cd attestor/scripts
node Transfer.js --devnet
```

This will output a block number and transaction hash. Use these values to create a query.

```sh
cd query-cli
cargo run -- \
  --cc3-rpc-url https://rpc.ccnext-devnet.creditcoin.network \
  --eth-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --contract-address 0x21cb3940e6ba5284e1750f1109131a8e8062b9f1 \
  --eth-rpc-url https://anvil.ccnext-devnet.creditcoin.network
```

Now you can wait for the prover to finish proving the query.

## Running the Prover in Light Mode

When run in light mode, the prover only schedules and provides inputs for proving jobs.
The actual proving work is delegated to an Azure data pipeline mediated by a
prover-be-api server. That server will soon be hosted on Kubernetes for devnet, but for
now you need to build and launch your own with docker.

To set up the prover-be-api server, clone the code base found [here](https://dev.azure.com/gluwa/Gluwa/_git/CCNext.StoneProver.BE). Then follow the steps in its readme [here](https://dev.azure.com/gluwa/Gluwa/_git/CCNext.StoneProver.BE?path=/CCNext.StoneProver.BE.API/README.md).

Note the exposed socket address of your prover-be-api server. In place of "http:// localhost:55644" below, use the socket exposed by your target prover-be-api instance.

In light mode you must also provide a UUID api key for requests sent to the prover backend server with the argument `--be-api-key`. Api keys are managed by the prover BE server administrator. So you need to ask them for a key. If you are launching your own BE server, then you need to look up or create a valid api key for your server.

// secretlint-disable

```sh
./target/release/prover \
--cc3-key "//Alice" \
--cc3-evm-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133" \
--name "devprover" \
--prover-be-socket-addr "https://cc-prover-api-dev-api.lemonpond-57fd618e.westus3.azurecontainerapps.io" \
--be-api-key "f40677cb-8aa5-4a8e-bb99-2933b12b473c" \
--postgres-uri "postgres://prover:prover@127.0.0.1:5432/attestations"
```

// secretlint-enable

When set up correctly, the light prover will send proving requests to the prover-be-api server. Then in a few minutes the server will respond with an output proof file.

When sending queries to BE instances not hosted locally, use an address prepended with "https://". EX: --prover-be-socket-addr "https:// 122.0.38.55:55644"

[enable flakes]: https://nixos.wiki/wiki/flakes#Enable_flakes_temporarily
[polkadot js]: https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944#/explorer
[`anvil`]: https://book.getfoundry.sh/reference/anvil/
[installation instructions]: https://book.getfoundry.sh/getting-started/installation
[step 4]: #4-make-a-transfer
