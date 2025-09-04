# Prover Smart Contract

The `sol` directory contains the `Prover.sol` smart contract, which is responsible for serving as public prover.

## Installation

```sh
npm install
```

## Generate Artifacts

```sh
./build.sh
```

## Testing

Testing for the `Prover.sol` smart contract is executed as part of the
`docs-smart-contract-development-with-hardhat:` CI job defined inside `.github/workflows/ci.yml`.

The entry-point for this testing is `npx hardhat test` and the test suite location is under
`docs/smart-contract-development/with-hardhat/test/`.
