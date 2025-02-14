# Universal Smart Contract POC

**This example uses the decentralized bridging capabilities of Creditcoin Next to allow for crosschain interactions in a Universal Smart Contract. The Universal Smart Contract (USC) in this example allows for simple burn + mint functionality, where an ERC 20 token is burned on a source chain such as Ethereum and then minted as a corresponding ERC 20 in the CC3 EVM.**

# Running the POC

## 1. Spin up Bridge POC
Universal smart contracts hosted on Creditcoin3-next are dependant on the CCNext decentralized bridge.

To stand up an instance of the CCNext bridge, follow steps 1-5 in [document](../poc.md)

## 2. Deploy ERC20 Smart Contract on Source Chain
A source chain is any chain which CCNext provides a bridge to. In our example the source chain is the anvil chain we stood up as a part of step 1.

We now deploy an ERC20 smart contract on our source chain. The contract has 
