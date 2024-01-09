# Creditcoin3

A Creditcoin3 node with the Ethereum RPC support, ready for deploying smart contracts.

## Generation & Upstream

This project was originally forked from the Frontier template. The template is maintained in the
[Frontier](https://github.com/paritytech/frontier/tree/master/template) project repository, and can
be used to generate a stand-alone template for use in an independent project via the included
[template generation script](https://github.com/paritytech/frontier/blob/master/docs/node-template-release.md).

A ready-to-use template generated this way is hosted for each Frontier release on the
[substrate-developer-hub/frontier-node-template](https://github.com/substrate-developer-hub/frontier-node-template)
repository.

This template was originally forked from the
[Substrate Node Template](https://github.com/substrate-developer-hub/substrate-node-template). You
can find more information on features on this template there, and more detailed usage on the
[Substrate Developer Hub Tutorials](https://docs.substrate.io/tutorials/v3/) that use this heavily.

## Build & Run

To build the chain, execute the following commands from the project root:

```bash
cargo build --release
```

To execute the chain, run:

```bash
./target/release/creditcoin3-node --dev
```

The node also supports to use manual seal (to produce block manually through RPC).
This is also used by the ts-tests:

```bash
$ ./target/release/creditcoin3-node --dev --sealing=manual
# Or
$ ./target/release/creditcoin3-node --dev --sealing=instant
```

### Docker Based Development

Optionally, You can build and run the frontier node within Docker directly.
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

## Genesis Configuration

The development [chain spec](node/src/chain_spec.rs) included with this project defines a genesis
block that has been pre-configured with an EVM account for
[Alice](https://docs.substrate.io/v3/tools/subkey#well-known-keys). When
[a development chain is started](https://github.com/substrate-developer-hub/substrate-node-template#run),
Alice's EVM account will be funded with a large amount of Ether. The
[Polkadot UI](https://polkadot.js.org/apps/#?rpc=ws://127.0.0.1:9944) can be used to see the details
of Alice's EVM account. In order to view an EVM account, use the `Developer` tab of the Polkadot UI
`Settings` app to define the EVM `Account` type as below. It is also necessary to define the
`Address` and `LookupSource` to send transaction, and `Transaction` and `Signature` to be able to
inspect blocks:

```json
{
  "Address": "MultiAddress",
  "LookupSource": "MultiAddress",
  "Account": {
    "nonce": "U256",
    "balance": "U256"
  },
  "Transaction": {
    "nonce": "U256",
    "action": "String",
    "gas_price": "u64",
    "gas_limit": "u64",
    "value": "U256",
    "input": "Vec<u8>",
    "signature": "Signature"
  },
  "Signature": {
    "v": "u64",
    "r": "H256",
    "s": "H256"
  }
}
```

Use the `Developer` app's `RPC calls` tab to query `eth > getBalance(address, number)` with Alice's
EVM account ID (`0xd43593c715fdd31c61141abd04a99fd6822c8558`); the value that is returned should be:

```text
x: eth.getBalance
340,282,366,920,938,463,463,374,607,431,768,211,455
```

> Further reading:
> [EVM accounts](https://github.com/danforbes/danforbes/blob/master/writings/eth-dev.md#Accounts)

Alice's EVM account ID was calculated using a utility script.

## Example 1: Deploy basic contract using Remix & Metamask

### Adding local network to Metamask

Creditcoin3 is compatible with most tooling from the Ethereum ecosystem, including browser wallets like Metamask. To connect to your local dev node, add it as a new network:

```text
Network name: Creditcoin3 Local
New RPC URL: http://127.0.0.1:9944
Chain ID: 42
Currency symbol: CTC
Block explorer URL: <empty>
```

### EVM accounts

To fund an account, simply transfer from one of the dev accounts. Import the Alith account to Metamask and you should see it funded with 1M CTC.

Alith (SUDO) keys:

```text
Address: 0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac
Private key: 0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133
```

### Deploying contracts with Remix

We will deploy a simple Counter contract:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.4;

contract TestCounter {
    int private count = 0;
    function incrementCounter() public {
        count += 1;
    }
    function decrementCounter() public {
        count -= 1;
    }

    function getCount() public view returns (int) {
        return count;
    }
}
```

1. Open the [Remix IDE](https://remix.ethereum.org/). In the Remix IDE, click on the Solidity tab and create a new file called Counter.sol. Paste the Counter contract in the file.

2. In the Remix IDE, click on the Deploy & Run tab. In the Environment dropdown, select Injected Web3. This will prompt you to connect to Metamask.

3. Click on Deploy and Metamask should prompt you to sign the transaction.

4. Once deployed, you can interact with the contract through the Deployed Contracts dropdown menu. Try sending transactions to increase and decrease the counter.
