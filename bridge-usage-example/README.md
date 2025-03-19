# Universal Smart Contract POC

**This example uses the decentralized bridging capabilities of Creditcoin Next to allow for crosschain interactions in a Universal Smart Contract. The Universal Smart Contract (USC) in this example allows for simple burn + mint functionality, where an ERC 20 token is burned on a source chain such as Ethereum and then minted as a corresponding ERC 20 in the CC3 EVM.**

# Running the POC

## 0. Setup
Get dependencies and build
```sh
cd bridge-usage-example
npm install
forge build
```

## 1. Spin up Bridge POC
Universal smart contracts hosted on Creditcoin3-next are dependant on the CCNext decentralized bridge.

To stand up an instance of the CCNext bridge, follow steps 1-5 in [document](../poc.md)

## 2. Deploy TestERC20 Smart Contract on Source Chain
A source chain is any chain which CCNext provides a bridge to. In our example the source chain is the anvil chain we stood up as a part of step 1.

We now deploy our TestERC20 smart contract on our source chain. The contract automatically funds its creator's address with 1000 TEST coins.

Run the following to deploy your contract:

// The `--private-key` provided is one of the default pre-funded Anvil accounts
```sh
cd bridge-usage-example
forge create --rpc-url 127.0.0.1:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 TestERC20
```

Upon successful contract creation, the resulting logs will contain your TestERC20 contract address. We will need this in the next step.
EX: "Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3"

## 3. Burn TestERC20 Tokens
Burn contract tokens by transferring them to one of the 0x0000000... addresses. Use the private key and contract address from step 2.

cast send --rpc-url <RPC-URL> <CONTRACT-ADDRESS> "transfer(address, uint256)" <ADDRESS> <AMOUNT> --private-key <PRIVATE-KEY>

Again, we use our pre-funded Anvil account with private key 0xac09...
EX:
```sh
cast send --rpc-url 127.0.0.1:8545 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0 "transfer(address, uint256)" "0x0000000000000000000000000000000000000001" "50" --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

## 4. Create a CCNext Proving Query
Follow step #7 in [document](../poc.md)

Provide the following responses to the CLI prompts:
Network: 3 (Local),
Block Height: "Block height of your transaction from step 3",
Transaction Hash: "Hash of your transaction from step 3",

The prover that you ran in step 1 should begin proving your transaction. This usually takes about 15 minutes. See prover logs to check progress. 

Note: If you are using an x86 machine, Linux or Mac, then you can run proof generation locally. Otherwise you will have to run the prover in light mode as detailed in step #10 of [document](../poc.md)

## 5. Deploy Universal Smart Contract
TODO: Add Universal Smart Contract (USC) example in this repo. It should be an OpenZeppelin ERC20 with one additional function, `usc_bridge_complete_mint(prover_contract_addr, query_id)`.

## 6. Mint Tokens Using Proof of Burn
We can see that proof generation is complete in either of two places:
1. The prover logs
2. The event `QueryVerified` which is visable on Polkadot.js

Once proof generation and submission is complete, a proof of the TestERC20 token burn transaction is present in the public prover contract. This contract is deployed on the CC3 EVM and its address can be found in `creditcoin3-next/artifacts/chain_deployment_artifact.json`.

We can now make a call on our Universal Smart Contract. The function signature of the call we want to make is:

```
usc_bridge_complete_mint(prover_contract_addr, query_id)
```

We provide the contract call with a `prover_contract_addr` and with a `query_id`.