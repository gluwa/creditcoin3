In order to generate proving queries for transactions involving our `TestERC20` contract, we need to provide the contract ABI to the query builder in the query CLI. The default `TestERC20` ABI should already be hard coded in `/query-cli/src/query_builder.rs`. But in the case of any changes made to `TestERC20` you can replace the existing hard coded ABI with the results generated here.

1. Generate the new ABI Json
```sh
cd bridge-usage-example
solc "src/TestERC20.sol" --combined-json abi --overwrite --json-indent 2 > TestERC20Abi.json
```

2. Condense Json
```sh
jq -c '.contracts["src/TestERC20.sol:TestERC20"].abi' < TestERC20Abi.json | jq -Rsa
```

3. Copy into `/query-cli/src/query_builder.rs`
Looks like:
```rs
let json_str = r#"[{"constant":false,"inputs":[{"n......
```