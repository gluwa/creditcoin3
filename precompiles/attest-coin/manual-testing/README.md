# Attestcoin Precompile Testing
A minimal set of steps to test the functionality of the attestcoin precompile.

## Steps List
1. Stand up local chain following steps 1-2 in .github/CONTRIBUTING.md
2. Configure pallet attestation 
    a. set_target_sample_size -> 1
    b. set_min_bond_requirement -> 100
3. Deploy Attestcoin ERC20 contract
4. Set Attestcoin rewards token in palletAttestcoinRewards
5. Fund Attestcoin precompile account, attestor stash, and attestor operator account using sudo
    a. With CTC, 100 each
    b. With ATC, 10,000 for the precompile and 100 for the attestor stash
6. Call `deposit` in the attestcoin precompile to fund a mapped EVM stash account with pallet assets attestcoin
7. Call `register_attestor` in the attestor stash precompile
8. Start your attestor, mostly following steps from https://docs.creditcoin.org/attestcoin-protocol/attestcoin-protocol-operator-guides/attestor-operator-guide 
9. Run script that listens for the first attestation to arrive on-chain, verifies the reward balance by calling `accrued` in the attestcoin precompile, then calls the attestcoin precompile function `claim` to claim the reward.
10. Call `chill` on our attestor using the attestor-stash precompile
11. Call `unregister_attestor` using the attestor-stash precompile then `withdraw` in the attestcoin precompile
12. Check the resulting attestcoin ERC20 balance in our EVM account


## Steps Details

### 1. Stand up local chain

Creditcoin node:
```sh
cargo build --features=fast-runtime --release
./target/release/creditcoin3-node --dev --tmp
```

Anvil node:
```sh
anvil --block-time 6
```

### 2. Configure Pallet Attestation

Point Polkadot.js at your local node: https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944#/explorer 

Go to Developer -> Sudo and select the call Attestation -> setTargetSampleSize. 
Params:
chainKey -> 2
newTargetSampleSize -> 1

Now go to Developer -> Sudo and select the call Attestation -> setMinBondRequirement.
Params:
chainKey -> 2
minBondRequirement -> 100 (with added 18 0's)

### 3. Deploy Attestcoin ERC20 Contract

From this directory:

```sh
npm install
node scripts/deploy-erc20.js
```

Output:
```text
RPC          http://127.0.0.1:9944 (evm chain id 42)
deployer     0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac
balance      1000000.0 CTC

deploying    0x7fbd849995d041fdc49a81e90c7afd7a18ee95c83f914890b792a9b98b4ebd5b
deployed     0x970951a12F975E6762482ACA81E57D5A2A4e73F4
minter       0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac
```

### 4. Set Attestcoin Rewards Token in PalletAttestcoinRewards

Point Polkadot.js at your local node: https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944#/explorer 

Go to Developer -> Sudo and select the call AttestCoinRewards -> setAttestCoinToken. 
Params:
token -> <ATTESTCOIN_ERC20 from .env>

### 5. Fund Accounts

5.1: Fund precompile account

- ATC funding

The mint is an EVM call signed with the `DEPLOYER_PRIVATE_KEY` from step 3. The `precompile`
alias resolves to `0x...0fd5` so you don't have to paste it:

```sh
node scripts/fund-erc20.js precompile 10000
```

5.2: Fund Attestor stash EVM account

- Create Stash EVM Account

```sh
node scripts/new-stash.js 
```

- CTC funding
In polkadot.js go to Developer -> Sudo and select the call Balances -> forceSetBalance. 
Params:
who -> Address20
Address20 -> <STASH_ADDRESS from .env>
newFree -> 10000000000000000000

- ATC funding

```sh
source .env
node scripts/fund-erc20.js $STASH_ADDRESS 100
```

The stash needs this because step 6's `deposit` pulls ERC-20 from the caller
with `transferFrom` before minting the pallet-assets attest coin that step 7
bonds.

5.3: Fund Attestor operator substrate account

- Create Attestor Substrate Account
```sh
OUT=$(subkey generate --output-type json); { printf '\n# Attestor operator account (sr25519), from `subkey generate`.\n'; printf 'ATTESTOR_SS58=%s\n' "$(echo "$OUT" | jq -r .ss58Address)"; printf 'ATTESTOR_SEED=%s\n' "$(echo "$OUT" | jq -r .secretSeed)"; } >> .env; grep ATTESTOR_ .env
```

- Fund the account
In polkadot.js go to Developer -> Sudo and select the call Balances -> forceSetBalance. 
Params:
who -> Id
Id -> <ATTESTOR_SS58 from .env>
newFree -> 10000000000000000000

### 6. Fund Mapped EVM Stash via Precompile

`deposit` pulls ERC-20 from the caller with `transferFrom` and mints the same
amount of asset 1 to the caller's *mapped* Substrate account,
`blake2_256("evm:" || address)`. That mapped account is the stash — the account
pallet attestation bonds from in step 7 and accrues rewards to.

The approve targets the precompile because the precompile is the approved
spender in its own `transferFrom` subcall. The script does both calls:

```sh
node scripts/deposit.js
```

Defaults to 100 ATC, matching the min bond set in step 2. Pass a different
amount as the first argument.

### 7. Call register_attestor in the attestorStash Precompile

This has to go through the precompile rather than a polkadot.js extrinsic. The
stash is `blake2_256("evm:" || address)` — a hash with no signing key — so it
can never sign an extrinsic. That is what the attestor-stash precompile is for.

Submits via the CLI's `attestor register` (same precompile); the script adds the
pre-flight checks the CLI lacks. Needs the built CLI and `STASH_MNEMONIC`, as in
step 10.

```sh
node scripts/register-attestor.js
```

### 8. Start your attestor

Build once, then run:

```sh
cargo build --release -p attestor
node scripts/run-attestor.js
```

If the attestor exits with a metadata mismatch:

```sh
subxt metadata --url ws://127.0.0.1:9944 --version 16 -f bytes > common/cc-client/artifacts/metadata.scale
cargo build --release -p attestor
```

### 9. Wait for attestation, then claim reward

```sh
node scripts/claim-rewards.js
```

Waits for the first `BlockAttested` / `CommitSignersRewarded`, reads `accrued`
through the precompile, then claims the full balance to the stash's EVM address.
Skips the wait if something has already accrued.

### 10. Chill, then Unregister Attestor When Idle

Wraps the creditcoin CLI, whose `attestor chill` / `attestor unregister` already
call the attestor-stash precompile. `chill` polls until the attestor is Idle,
which `unregister_attestor` requires.

```sh
cd ../../../cli && yarn install && yarn build && cd -
node scripts/chill-and-unregister.js
```

### 11. Call Withdraw Unbonded and Withdraw ATC to EVM

Two moves, the first unblocking the second:

1. `attestor withdraw-unbonded` (CLI, stash precompile) — returns elapsed chunks
   from the bond pool to the stash's liquid pallet assets balance, reaping the
   ledger once nothing is bonded
2. `withdraw` on the attest-coin precompile — burns that liquid attest coin and
   sends the same amount of ERC-20 back. No CLI equivalent; the `attest-coin`
   group is read-only.

```sh
node scripts/withdraw.js
```

### 12. Check Resulting EVM Balance

```sh
node scripts/show-balances.js
```

Read-only. After a full run the stash holds nothing bonded, nothing liquid and
nothing unclaimed, and its ERC-20 balance is the rewards claimed plus the bond
that round-tripped back out.













