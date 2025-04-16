In order to generate proving queries for transactions involving our `TestERC20` contract, we need to provide the contract ABI to the query builder in the query CLI. The default `TestERC20` ABI should already be in `TestERC20Abi.json`. But in the case of any changes made to `TestERC20` you can replace the existing hard coded ABI with the results generated here.

1. Generate the new ABI Json
```sh
cd bridge-usage-example
solc "src/TestERC20.sol" --combined-json abi --overwrite --json-indent 2 | jq -c '.contracts["src/TestERC20.sol:TestERC20"].abi' > TestERC20Abi.txt
```