# Attestation network description

This document will explain the inner workings of the attestation network and it's limitations.

## Components

- Attestation Pallet `(./pallets/attestation-poc)`
- Client `./client/attestor-gossip`
- Client RPC `./client-attestor-rpc`
- Node (creditcoin3-next node)
- Attestor `./attestor`

### Attestation pallet

This pallet has multiple responsibilities regarding the attestation network, a summary is:

- (Un)Registering Attestors
- Setting committee size (min number of attestors).
- Setting of supported chains
- Inherent for submitting a signed attestation (signed by the entire network)

So it is part a registry as a provider for the attestation data on chain, the attestations are linked to eachother with a digest, this digest is calculated client side by validators.

A sudo key can be used to set the committee size, this is the size of the minimal number of votes that the attestors should provide for a given block.

Also the sudo key can be used to add support for a chain by ID.

It provides an inherent transaction for validators in order to create a valid attestation for a block.

### Client

Attestor gossip client is a package that provides functionality to gossip peer to peer about votes for an attestation on a source chain. This connects all the nodes in the network using the gossip protocol. It validates all incoming votes and triages them accordingly, when a vote is validated it is gossiped through the network. When enough votes (> committee size) is reached then it uses BLS to combine all the valid attestors signatures. Once that is done, the attestation is submitted to the inherent data provider and a validator will submit this inherent data.

### Client RPC

Defines an RPC interface for submitting an attestation by the attestor network.

### Node

Creditcoin3 next node, it runs the attestor gossip network alongside it's normal operations. Nothing special is required for node operators. Probably it should only run the attestor gossip network if the node is running as a validator.

### Attestor

The attestor binary. This is a program that listens to a source Chain RPC endpoints and gets notified when a block is created. Once a block is created, the program will sign that data and sign the VRF output. It will calculate it's inclusion in the network and depending on the outcome will submit the signed attestation to the attestor gossip network of a creditcoin3 node. This can be a local or remote creditcoin3 node.

Once the binary is started, it will register itself on creditcoin3 as an attestor, based on the VRF output it can either include it's attestations or not (see research for more details about inclusion).

## Open question & limitations

- Attestor gossip protocol cannot blacklist / slash a faulty attestor.
- Only one signed attestation can be handled each block by the validators. So the queue can grow infinite in the meanwhile.
- Stale votes for blocks are never purged
- Votes for blocks that are attested for already are accepted but never purged.
- What should be the attestor lag (to not spam the network)?
- Attestations for future blocks of source chain when creditcoin3 lags behind are kept in memory but probably need to be only submitted when the previous attestations are submitted as well.
- Can we batch attestations in the inherent transaction?
