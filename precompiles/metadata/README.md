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
cat sol/proof_verifier.sol | jq -Rs '.'
```

## 5. Copy Resulting Text Into Final Json

Resulting line in `precompiles-creditcoin3-devnet.json` should look like:
```json
"source": "// SPDX-License-Identifier: GPL-3.0-only\npragma solidity >=0.8.3;
```

## 6. Check Json was Updated Successfully

```sh
cp .github/check-solidity-source-vs-metadata.sh .
./check-solidity-source-vs-metadata.sh
```

## 7. Maybe remove extra newline if check fails

There may be an extra newline at the end of your new `source` line. It would look like 
```json
"source": "// SPDX-License-Identifier: GPL-3.0-only...........external returns (ResultSegment[] memory);\n}\n"
```
If so, then remove the extra new line, resulting in
```json
"source": "// SPDX-License-Identifier: GPL-3.0-only...........external returns (ResultSegment[] memory);\n}"
```