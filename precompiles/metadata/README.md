# Steps for Updating Metadata After Precompile Changes

## 1. Run abi creator 

```sh
cd precompiles/metadata
./abi-creator.sh
```

## 2. Condense the Json File of the Changed Precompile to a Single Line

```sh
jq -c '.contracts["sol/proof_verifier.sol:QueryVerifierContract"].abi' < abi/proof_verifier.json | jq -Rsa
```

## 3. Copy Resulting Text Into Final Json

Resulting line in `precompiles-creditcoin3-devnet.json` should look like:
```json
"abi": "[{\"inputs\":[{\"internalType\":\"QueryId\"......
```

## 4. Condense Source Code to Single Line

```sh
jq -Rs '.' sol/proof_verifier.sol
```

## 5. Copy Resulting Text Into Final Json

Resulting line in `precompiles-creditcoin3-devnet.json` should look like:
```json
"source": "// SPDX-License-Identifier: GPL-3.0-only\npragma solidity >=0.8.3;
```