# Scripts

## Install

```bash
npm install
```

## Run Transfer

Will transfer some amount from `Anvil's Account #0` to some other random account.

```bash
node Transfer.js
```

## Transfering Checkpoints Between CCNext Instances

The establishment of checkpoints attesting to a source chain can take quite a long time.
EX: Sepolia ingestion (creation of checkpoints) took ~10 days.

For the purpose of standing up new CCNext chains it can be helpful to transfer the
checkpoints for a source chain instead of re-doing the ingestion work.
EX: CCNext Devnet -> Sepolia checkpoints transfer -> CCNext Testnet

Steps:

### 1. Create .env file

Create file `attestor/scripts/.env` with contents:
```
MNEMONIC=<your sudo mnemonic here>
SOURCE_CHAIN=<source chain web socket url here>
DESTINATION_CHAIN=<target chain web socket here>
CHAIN_KEY_ON_SOURCE=<chain key of chain for which to copy checkpoints>
CHAIN_KEY_ON_DESTINATION=<chain key of chain to insert checkpoints into>
```
Note: The mnemonic stored here should correspond to an account with sudo access
on the target chain.

EX:
```sh
MNEMONIC="//Alice"
SOURCE_CHAIN="wss://rpc.ccnext-devnet.creditcoin.network"
DESTINATION_CHAIN="ws://127.0.0.1:9944"
CHAIN_KEY_ON_SOURCE=2
CHAIN_KEY_ON_DESTINATION=2
```

### 2. Run ExportCheckpoints.js

```sh
cd attestor/scripts
node ExportCheckpoints.js
```

Check the generated file `checkpoints.csv` to see if contents look right:
EX:
```
675550,0x04130ca64dccc93140ac564515f1e00bf0e0d6c5436accd113ff6c954e83321f
675650,0x07b7f0095c8764d1fb3a156861c8ff499f94ed4af39ff499064d4c3a5d205ad6
```

Each row is `block_number,digest_hex` entry. Rows are sorted ascending by
`block_number`; `ImportCheckpoints.js` reverses them internally so checkpoints are
submitted newest-to-oldest.

### 3. Run ImportCheckpoints.js

With `CHECKPOINTS_FILE=checkpoints.csv` (plus `MNEMONIC`, `DESTINATION_CHAIN`, and
`CHAIN_KEY_ON_DESTINATION`) set in `attestor/scripts/.env`:

```sh
node ImportCheckpoints.js
```

Configuration may also be passed via CLI arguments, which take priority over env vars:

| CLI argument  | Env variable               | Description                                    |
|---------------|----------------------------|------------------------------------------------|
| `--file`      | `CHECKPOINTS_FILE`         | Path to the CSV file                           |
| `--rpc`       | `DESTINATION_CHAIN`        | WebSocket URL of the target chain              |
| `--chain-key` | `CHAIN_KEY_ON_DESTINATION` | Chain key to import checkpoints into           |
| *(none)*      | `MNEMONIC`                 | Sudo account mnemonic (required, env var only) |

```sh
node ImportCheckpoints.js --file checkpoints.csv --rpc ws://127.0.0.1:9944 --chain-key 2
```

Checkpoints are submitted in batches of 100 via a `sudo(attestation.importCheckpoints(...))` call.

Running in Development Mode

You can pass the --dev flag to reduce the retry delay from the default 15000ms (15s) to 6000ms (6s).
This makes iteration faster when testing locally.

```sh
node ImportCheckpoints.js --dev
```

### 4. (Optional) Check That On-chain Checkpoints Match Expected

Modify and re-run ExportCheckpoints.js to extract the checkpoints we just inserted into checkpoints2.csv.
Then check that the contents of checkpoints.csv exactly match those of checkpoints2.csv

Change the file written to on this line to `checkpoints2.csv`:
```js
fs.writeFileSync('checkpoints.csv', csv);
```

Change `SOURCE_CHAIN` in .env to match `DESTINATION_CHAIN`:
```
SOURCE_CHAIN="ws://127.0.0.1:9944"
DESTINATION_CHAIN="ws://127.0.0.1:9944"
```

Export checkpoints:
```sh
node ExportCheckpoints.js
```

Compare files:
```sh
diff checkpoints.csv checkpoints2.csv
```
On success no file differences will be printed

### 5. Attest On Top of Imported Checkpoints

No special steps here. Simply register and run attestors as normal for the chain_key you
imported checkpoints to.

The resulting attestor sync process should take a much shorter time than it otherwise would.
