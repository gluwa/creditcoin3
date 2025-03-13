# Creditcoin3

A Creditcoin3 node with the Ethereum RPC support, ready for deploying smart contracts.

## Dev environment setup

To install tools & binaries used during development execute:

```bash
cargo install taplo-cli --locked
```

To install git hooks, which will stop you from committing common mistakes,
from the root directory of this repository execute:

```bash
ln -s ../../.github/hooks/pre-commit .git/hooks/pre-commit
ln -s ../../.github/hooks/pre-push .git/hooks/pre-push
```

## Build & Run

To build the chain, execute the following commands from the project root:

```bash
cargo build --release
```

To execute the chain, run:

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
