# Writability POC components

This document describes the components required for the writability (attestcoin) POC, it oulines their functionaly and also proposes some questions regarding production versions.

## SimpleOutboxContract

Receives message relay requests through `deliverMessage` called from dApp, for the POC called from a script like `publishMessage` or similar.

Can use https://github.com/gluwa/usc-write-ability-research/blob/contracts-dev/contracts/Outbox.sol as basis for it.

Caller needs to pay for the message to be published.

## SimpleQuoter

Offchain HTTP service that receives quoting requests from dApp and returns them for the dApp to submit to the relayer contract.

### Questions

* In order for the relayer to validate the quotes, some form of registration seems to be necessary in the relayer contract?

## Attester

Simple offchain worker which listens to the `SimpleOutboxContract` for `MessagePublished` events and votes on them.

Once message is voted call HTTP endpoint in relayer with message to deliver.

### Questions

* Should they be integrated into the current attestor codebase?
* If part of the attestors, voting should happend in a similar manner attestions are voted?

## DummyRelayerContract

Simple contract which accepts the quotes submitted from the dApp

### Questions

* How can we enforce that only a single relayer contract will be created for a given destination chain?
    * If not enforced, how does it work when two relayer contracts have relayers for the same destination network?
* How can a relayer know when to distribute rewards to it's linked relayers?

## SimpleRelayer

Offchain worker who picks up voted messages from the relayers and calls the destination inbox contract with the message payload.

Additionally relayers also listen for `MessageDelivered` events for messages that require ack and then call the `acknowledge` in the `SimpleOutboxContract`

Since relayers end up calling the destination contract they need enough gas in the destination chain to actually relay the messages.

### Questions

* How can relayers ensure that messages they relay have been paid for?
* How do relayers listen for voting results? Do they live in the same P2P network as attesters?
* If the relayer determines that the cost is higher than the original reward, should it be able to request a new quote itself?

## SimpleInboxContract

Contract that receives messages from the relayers, validates them and the tries to forward them to the destination contract

In order to validate message votes, contract must expose method to register attesters

Can use https://github.com/gluwa/usc-write-ability-research/blob/contracts-dev/contracts/Inbox.sol as basis for it.

## DummyTargetContract

Simple contract used to highlight event received, must have a callable function `receiveMessage`

## Simple mock dApp

Simple script that will create and send messages to be relayed, listen to target contract and to acknowledgment events.

Maybe we can implement it as an HTTP server or just as a simple listener and use a separate script to send messages.

# Summary

Of all the components the `SimpleQuoter` and `DummyRelayerContract` don't seem to fit easily in the POC. Since there doesn't seem to be a way for the relayers to actually know if a given message has been "paid" for.

One approach that comes to mind is that the caller would first call the `deliverMessage` in `SimpleOutboxContract` and the note the `messageId` emited, with that he would call the `SimpleQuoter` which would attach a quote to that messageId valid for 
a given number of blocks. Then submit that quote to the `DummyRelayerContract` which would then emit an event that the `SimpleRelayer` would read marking that messageId as paid, so that when the message has been voted the relayers know they can attempte delivery.

Regarding `ack` flows it seems necessary for the attesters/relayers or whatever other component to list to the `SimpleInboxContract` for the `MessageDelivered` event and then from that call the `SimpleOutboxContract.acknowledge` function.

# Phase 1

## Bradley
* Simple mock dApp (listener + script)

## Didac
* Attester
