# Creditcoin3

A Creditcoin3 node with the Ethereum RPC support, ready for deploying smart contracts.

## Build & Run

To build the chain, execute the following commands from the project root:

```bash
cargo build --release
```

To execute the chain, run:
error:
Service(Client(Storage("wasm call error Other: Exported method GenesisBuilder_get_preset is not found")))

need to fix with
https://substrate.stackexchange.com/questions/10690/building-a-chain-specification-with-raw-gives-me-wasm-call-error-other-expo

```bash
./target/release/creditcoin3-node --dev
```

_WARNING: running natively on Windows [is unsupported](https://github.com/gluwa/creditcoin/security/advisories/GHSA-cx5c-xwcv-vhmq)._

The node also supports to use manual seal (to produce block manually through RPC).
This is also used by the ts-tests:

```bash
$ ./target/release/creditcoin3-node --dev --sealing=manual
# Or
$ ./target/release/creditcoin3-node --dev --sealing=instant
```

### Docker Based Development

Optionally, you can build and run creditcoin3-node within Docker directly.
The Dockerfile is optimized for development speed.
(Running the `docker run...` command will recompile the binaries but not the dependencies)

Building (takes 5-10 min):

```bash
docker build -t creditcoin3-node-dev .
```

Running (takes 1 min to rebuild binaries):

```bash
docker run -t creditcoin3-node-dev
```
