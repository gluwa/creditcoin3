# Query result checker

## Installation

```bash
yarn
```

## Update Prover ABI
```bash
cd ..
./build.sh
```
Then cut and paste the full abi array into `get-proof-results/prover-abi.json`
Should look like:
```
[
    {
      "inputs": [
        {
          "internalType": "address",
          "name": "_proceedsAccount",
          "type": "address"
        },
      ]
    }
    ...
    ...
    ...
]
```

## Usage

Edit the `index.js` file to set the `HTTPS_RPC_URL` to your Creditcoin node.

```bash
node index.js <proverContractAddress> <queryId>
```
