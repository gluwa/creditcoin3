## Before we get started

In this document we will explain how to run `cc3-next` locally and verify blockchain state from
a source chain. This is a fully featured example on how to run your own local execution chain,
source chain, and attestation network, then use the native query verification precompile to prove
state availability.

## Definitions

First, some definitions:

- **Execution chain**: Decentralized oracle (`cc3-next`) responsible for aggregating state proofs
  from a _source chain_.
- **Source chain**: Chain emitting state which is synchronized and proven to the _execution chain_.
  For demonstration purposes we will be using Ethereum as the source chain.
- **Attestation network**: Decentralized network of nodes responsible for aggregating information
  about the state of a _source chain_: attestors bridge information about that chain's state to the
  _execution chain_.
- **Query CLI**: A command-line tool that uses the native query verification precompile (0x0FD2)
  to verify state from a _source chain_ on the _execution chain_. The verification happens
  on-chain using optimized native code, eliminating the need for separate prover services.
- **Native Query Verifier Precompile**: An EVM precompile at address 0x0FD2 that provides
  gas-efficient query verification directly in the execution chain runtime.

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
> If you are using `nix`, most setup steps are automated for you.

Before running the commands below, ensure you have all the required dependencies installed.
The query-cli and attestor binaries will be built automatically when you build the project.

## Resetting After Tests

Whenever you start up a new chain, you should clean up any artifacts from previous runs:

```bash
# Remove chain deployment artifacts
rm -rf artifacts/chain_deployment_artifacts.json

# If using Alice's default dev account, remove the blockchain database
rm -rf ./alice-data
```

## 0. Build the project

Before anything else, start by building the workspace in release mode.

```sh
cargo build --features=fast-runtime --release
```

This will build:
- `creditcoin3-node` - The blockchain node
- `attestor_zombienet` - The attestation network runner
- `query-cli` - The query verification CLI tool

## 1. Start local execution chain

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

[`anvil`] is a CLI tool which allows you to run an Ethereum local network to simulate _source chain_
transactions. Anvil is part of foundry, see [installation instructions].

Once anvil is installed on your system, start it with a 6-second block time to match Ethereum:

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#anvil
> ```
> If you get any errors, remember to **[enable flakes]**.

```sh
anvil --block-time 6
```

Anvil will start and display several pre-funded accounts with their private keys. The default
RPC endpoint is `http://localhost:8545`.

## 3. Start attestor zombienet

Now that we have a local _execution chain_ and _source chain_ set up, let's get our _attestors_ up
and running.

The attestor binary depends on a configuration file. A default configuration is available under
`attestor/config.yaml`. You can use this or create your own depending on your network needs.

Now you can start the attestor zombienet:

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#zombienet
> ```
> If you get any errors, remember to **[enable flakes]**.

```bash
./target/release/attestor_zombienet           \
    -n 3                                    \
    --bin=./target/release/attestor         \
    --eth-url=ws://localhost:8545           \
    --cc3-url=ws://localhost:9944           \
    --funding-address='//Alice'             \
    --config=./attestor/config.yaml
```

Now check if attestations are coming through on the polkadot explorer. You should see events like
`AttestationSubmitted` visible on the right panel of the block explorer. These attestations
confirm that the _source chain_ state is being synchronized to the _execution chain_.

## 4. Reading attestor logs

Individual zombienet attestor logs are stored under the `./logs` folder as raw JSON objects. It is
recommended to use a JSON log parser such as [hl] to obtain a more human-readable output.

To follow the output of new logs on zombienet attestor 0 with `hl`, run:

```bash
tail --retry -f ./logs/attestor-zombie-0.json.$(date -u +%Y-%m-%d-%H) | \
    hl -P -l i -h spans -h filename -h line-number
```

You can also view logs in a non-interactive way, to parse through debug information for example:

```bash
hl -h spans -h filename -h line-number ./logs/attestor-zombie-0.json.$(date -u +%Y-%m-%d-%H)
```

For more advanced used cases, make sure to check out the [hl documentation].

## 5. Make a transfer

We need some data on our _source chain_ for our _attestor zombienet_ to send over to our _execution
chain_. We can do this by sending a transaction to our local `anvil` chain.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#transfer
> ```
> If you get any errors, remember to **[enable flakes]**.

To send a native token transfer, run:

```bash
cd attestor/scripts
node Transfer.js
```

This will output something like:

```
Transfer transaction hash: 0x40a1f381b5eae8b86ada7cc1faf47ef22198190672e3ddd002933908eb49cd3a
Confirmed in block: 5
```

**Copy the block number and transaction hash.** You will need them in the next step.

## 6. Verify the query using query-cli

This is where everything comes together! We'll use the query-cli to verify that the transaction
you just made on the _source chain_ has been properly attested to the _execution chain_, and we
can prove its existence using the native query verification precompile.

Before we continue, wait for attestations to arrive. Look at your attestor output and wait for
something like:

```bash
INFO 📝 Received a new attestation: chain: 2, blocknumber: 20
```

Where `blocknumber` should be **greater than or equal to** the block number from [step 5].
This ensures the block has been attested before we try to verify it.

> [!IMPORTANT]
> If you are using `nix`, simply run:
> ```bash
> nix run .#query
> ```
> If you get any errors, remember to **[enable flakes]**.

Now run the query-cli in verify mode:

```sh
./target/release/query-cli \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --cc3-rpc-url ws://localhost:9944 \
  verify
```

The CLI will prompt you interactively:

```
Please select the network:
1. Sepolia
2. Ethereum
3. Local
4. Custom (provide ID and URL)
Enter your choice (1, 2, 3 or 4): 3
```

Select **3** for Local.

```
Enter local network URL (EX: ws://localhost:8545):
```

Press Enter to use the default `ws://localhost:8545`.

```
Enter the block height (number): 5
```

Enter the block number from step 5.

```
Enter the transaction hash: 0x40a1f381b5eae8b86ada7cc1faf47ef22198190672e3ddd002933908eb49cd3a
```

Paste the transaction hash from step 5.

```
Which data do you want represented in your proof results?
1. All data
2. Range of data
3. ERC20 transfer data
4. Native token transfer data
Enter your choice (1, 2, 3, or 4): 4
```

Select **4** for Native token transfer data.

### What happens next

The query-cli will:

1. **Fetch the block** from your local Anvil chain
2. **Generate a Merkle proof** for the transaction
3. **Generate a continuity proof** showing the chain of blocks
4. **Call the native query verifier precompile** (at address 0x0FD2) on the execution chain
5. **Display gas estimation** comparing native precompile vs Solidity contract costs
6. **Show the verification results** with extracted data segments

You should see output like:

```
=== Native Query Execution ===
Fetching block 5 from Local("ws://localhost:8545")...
Block fetched successfully

Query ID: 0xceafced73c2df93baf06d50b3e29438ad1c835423d7590782d3de72d1b2a4cca
Query details: Query { chain_id: 2, height: 5, layout_segments: [...] }

=== Block Structure ===
Block number: 5
Total transactions in block: 1
  Transaction 0: 976 bytes

=== Merkle Proof Generation ===
Merkle root: 0xf98e7271c6c94158bcc2fca85bad343134e2c5057b3f34db2a0bb20b45c7a63a
Siblings count: 0
Transaction data size: 976 bytes

=== Continuity Proof Generation ===
Fetching continuity chain from block 1 to 5
Constructed continuity proof with 5 blocks
Continuity blocks: 5

=== Query Verification ===

⛽ Gas Estimation:
   Total gas units: 109392
   ─────────────────────────────────────
   Estimated costs:
     0.001094 ETH at 10 gwei (low)
     0.002188 ETH at 20 gwei (avg)
     0.005470 ETH at 50 gwei (high)
     0.0109 ETH at 100 gwei (very high)

   Comparison with Solidity smart contract:
     Native Precompile (0x0FD2): 109392 gas
     Solidity Contract (est.): ~270000 gas

   Note: Native precompile provides optimized
         verification with reduced gas costs

✅ Verification successful!
Result segments count: 4
  Segment 0: offset=479, bytes=0x0000000000000000000000000000000001000000000000000000000000000000
    (Possible address: 0x0000000001000000000000000000000000000000)
  Segment 1: offset=223, bytes=0xe51aad88f6f4ce6ab8827279cfffb9226600000000000000000000000065941a
  Segment 2: offset=255, bytes=0x89968644a3c6532787d6bec3ae12116d8e000000000000000000000000000000
  Segment 3: offset=287, bytes=0x0000000000000000000b78adb8cfef3a09000000000000000000000000000000
```

**Congratulations!** You've successfully verified blockchain state from your _source chain_
(Anvil) on your _execution chain_ (cc3-next) using the native query verification precompile.
This demonstrates a **trustless, decentralized oracle** with on-chain verification.

## Understanding the Results

The result segments contain the extracted data from your transaction:

- **Segment 0**: Receipt status (success/failure)
- **Segment 1**: Transaction sender/recipient address data
- **Segment 2**: Transaction value or additional address data
- **Segment 3**: Gas used or other transaction metadata

The exact interpretation depends on the transaction type and the layout segments requested.

## Key Differences from the Old Approach

The native query verifier precompile approach offers several advantages:

1. **No separate prover service needed** - Verification happens directly on-chain
2. **Lower gas costs** - Native code is more efficient than Solidity contracts
3. **Simpler architecture** - No prover contract deployment or database management
4. **Immediate verification** - No waiting for off-chain proving jobs
5. **Stateless** - No need to track query state in a smart contract

The precompile at address `0x0FD2` handles all verification logic natively in the runtime,
making it faster and more cost-effective.

# Advanced

This section contains extra information on more advanced topics.

## Verifying ERC20 Transfers

To verify an ERC20 transfer instead of a native token transfer:

1. Deploy an ERC20 contract to Anvil
2. Make an ERC20 transfer
3. When prompted by query-cli, select option **3** (ERC20 transfer data)

The precompile will extract ERC20-specific data like sender, recipient, and transfer amount.

## Verifying Custom Data Ranges

To extract custom data from a transaction:

1. When prompted by query-cli, select option **2** (Range of data)
2. Specify the byte offsets and sizes you want to extract

This is useful for verifying arbitrary data from custom smart contract transactions.

## Using the Query CLI Programmatically

You can also use the query-cli in non-interactive mode by providing all parameters:

```sh
./target/release/query-cli \
  --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --cc3-rpc-url ws://localhost:9944 \
  verify \
  --network local \
  --network-url ws://localhost:8545 \
  --block-height 5 \
  --tx-hash 0x40a1f381b5eae8b86ada7cc1faf47ef22198190672e3ddd002933908eb49cd3a \
  --data-type native
```

(Note: Check `query-cli --help` for the exact command-line options available)

## Testing Against Devnet

To verify transactions on the Creditcoin devnet:

```sh
./target/release/query-cli \
  --cc3-evm-private-key "<your-private-key>" \
  --cc3-rpc-url wss://rpc.ccnext-devnet.creditcoin.network \
  verify
```

Then select the appropriate network (Sepolia or Ethereum) when prompted.

## Monitoring Gas Costs

The query-cli provides detailed gas estimation for each verification. You can use this to:

- Compare costs between different query types
- Estimate operational costs at different gas prices
- Optimize your query layout segments for efficiency

The native precompile typically uses 40-60% less gas than equivalent Solidity contract implementations.

[enable flakes]: https://nixos.wiki/wiki/flakes#Enable_flakes_temporarily
[polkadot js]: https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944#/explorer
[`anvil`]: https://book.getfoundry.sh/reference/anvil/
[installation instructions]: https://book.getfoundry.sh/getting-started/installation
[step 5]: #5-make-a-transfer
[hl]: https://github.com/pamburus/hl
[hl documentation]: https://github.com/pamburus/hl?tab=readme-ov-file#features-and-usage
