# Attestor POC

Document explaining how to setup and run the attestation POC

## Requirements

0. Install prerequisites
   - `cc` (`build-essential` package on ubuntu)
   - `protoc` (`protobuf-compiler` package on ubuntu)
   - `libclang` (`clang` package on ubuntu)

1. [Install the rust toolchain](https://www.rust-lang.org/tools/install).

2. Compile creditcoin `cd .. && cargo build --features fast-runtime`

3. Grab a zombienet binary from
  [here](https://github.com/paritytech/zombienet/releases/tag/v1.3.99) and put
   it somewhere (for example in the `zombie` directory).

4. [Install subkey](https://docs.substrate.io/reference/command-line-tools/subkey/) To generate keys.

## Setup

### Run Creditcoin

Start the network. This will create a network of 5 nodes running on your
   local system (all peered with each other).

```bash
cd zombie
./zombienet-macos spawn network.yaml -c 2 -l text
```

You should see output like

```log
All relay chain nodes spawned...
Namespace : zombie-5744a32a98ac758a71a31b67c26762e3
Provider : native
Node Information
Name : alice
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9944#/explorer
Prometheus Link : http://127.0.0.1:9615/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/alice.log
Node Information
Name : bob
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9945#/explorer
Prometheus Link : http://127.0.0.1:9616/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/bob.log
Node Information
Name : charlie
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9946#/explorer
Prometheus Link : http://127.0.0.1:9617/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/charlie.log
Node Information
Name : eve
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9948#/explorer
Prometheus Link : http://127.0.0.1:9619/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/eve.log
Node Information
Name : dave
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9947#/explorer
Prometheus Link : http://127.0.0.1:9618/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/dave.log
Node Information
Name : ferdie
Direct Link : https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9949#/explorer
Prometheus Link : http://127.0.0.1:9620/metrics
Log Cmd : tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/ferdie.log
```

Follow the logs of one (or more) of the nodes by copying one of the `Log Cmd`s from the output in the previous step. For instance, for the `bob` node's logs:

```bash
tail -f  /var/folders/jw/4ykz4cmj7q7fkjp9t6pv6z7h0000gn/T/zombie-5744a32a98ac758a71a31b67c26762e3_-56651-wDhqPf9AH6Z3/bob.log
```

You should see the logs updating in real time, with new log messages
appearing every ~6 seconds as new blocks are produced.

```log
2024-01-11 10:18:51 💤 Idle (5 peers), best: #1 (0xc72a…9431), finalized #0 (0xec32…d34d), ⬇ 8.1kiB/s ⬆ 9.0kiB/s
2024-01-11 10:18:54 ✨ Imported #2 (0x1e71…18cb)
2024-01-11 10:18:56 💤 Idle (5 peers), best: #2 (0x1e71…18cb), finalized #0 (0xec32…d34d), ⬇ 8.2kiB/s ⬆ 7.8kiB/s
2024-01-11 10:19:00 🙌 Starting consensus session on top of parent 0x1e71a7e5f049aaec29866fba0f189c79125315882fcda154a83fce5674f518cb
2024-01-11 10:19:00 🎁 Prepared block for proposing at 3 (0 ms) [hash: 0xd05038108a35ec7ef2e11426dc778d81d811e3b46b0ecd979a27d9efbced1ce1; parent_hash: 0x1e71…18cb; extrinsics (1): [0xe563…7303]
2024-01-11 10:19:00 🔖 Pre-sealed block for proposal at 3. Hash now 0x9bc3bdd635e1ca6698a8c63b6d6ffb1cf34edc9f1ee693942a879d69d024057b, previously 0xd05038108a35ec7ef2e11426dc778d81d811e3b46b0ecd979a27d9efbced1ce1.
2024-01-11 10:19:00 ✨ Imported #3 (0x9bc3…057b)
2024-01-11 10:19:01 💤 Idle (5 peers), best: #3 (0x9bc3…057b), finalized #1 (0xc72a…9431), ⬇ 9.9kiB/s ⬆ 9.7kiB/s
2024-01-11 10:19:06 ✨ Imported #4 (0x7c6c…7a4e)
2024-01-11 10:19:06 💤 Idle (5 peers), best: #4 (0x7c6c…7a4e), finalized #1 (0xc72a…9431), ⬇ 6.5kiB/s ⬆ 7.6kiB/s
^@2024-01-11 10:19:11 💤 Idle (5 peers), best: #4 (0x7c6c…7a4e), finalized #2 (0x1e71…18cb), ⬇ 9.0kiB/s ⬆ 8.5kiB/s
2024-01-11 10:19:12 ✨ Imported #5 (0x51e1…1fb4)
2024-01-11 10:19:16 💤 Idle (5 peers), best: #5 (0x51e1…1fb4), finalized #3 (0x9bc3…057b), ⬇ 8.3kiB/s ⬆ 8.0kiB/s
2024-01-11 10:19:18 ✨ Imported #6 (0x2bda…18ed)
```

### Anvil node

Open a pane for a local EVM network simulation

- Install [foundry](https://book.getfoundry.sh/getting-started/installation)

Run `anvil`

(Deprecated hardhat because it's not supporting `eth_getTransactionReceipt` rpc call)

### Run Attestors

We have a couple of prefunded attestors to start with. To start them you can the run the following commands from the attestor directory. Depending on the `THRESHOLD` variable in `client/atestor-gossip/src/worker.rs` file, you need to start 
atleast that number of attestors. For example, if the `THRESHOLD` is set to 3, you need to start 3 attestors minimum to be able to attest a block. 

#### Start the first prefunded attestor

```sh
cargo run -- -v --cc3-key "snake adult despair divide embrace this smart fatigue wine latin page parade" --dev
```

#### (Optionally) Start the second prefunded attestor

```sh
cargo run -- -v --cc3-key "silver mixed elevator layer copper venture taste also peanut evolve grab inquiry" --dev

```

#### (Optionally) Start the third prefunded attestor

```sh
cargo run -- -v --cc3-key "put badge smooth surround hawk today fortune like rigid exist village sphere" --dev
```

Alternatively, you can generate your own keys, transfer some balance to that address in order to register an attestor and then start the attestors.

```sh
subkey generate
```

Will output a new keypair, now use polkadotJS to connect to your local node and send some tokens to this address.

#### Start a first attestor

```sh
cargo +nightly run -- --cc3-key "your private key here" -v
```

#### (Optionally) Start a second attestor

Also generate a new key + transfer some balance.

```sh
cargo +nightly run -- --cc3-key "your private key here" -v
```

Look at the logs to see that the other nodes received it. Expect a line in the
log output like

```log
2024-01-11 10:19:21 Received message: Attestation(Attestation { round: 321, header_hash: 0x123456789abcdeff123456789abcdeff123456789abcdeff123456789abcdeff, header_number: 999, attestor: AttestorId(109876), topic: Topic(12345) })
```

### Do a transfer

```sh
cd ../hardhat
npx hardhat --network localhost run scripts/AutoTransfers.js
```
