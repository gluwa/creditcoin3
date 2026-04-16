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

Check the generated file `checkpoints.json` to see if contents look right:
EX:
```json
"0x07b7f0095c8764d1fb3a156861c8ff499f94ed4af39ff499064d4c3a5d205ad6": {
    "block_number": 675650
},
"0x04130ca64dccc93140ac564515f1e00bf0e0d6c5436accd113ff6c954e83321f": {
    "block_number": 675550
},
```

### 3. Run ImportCheckpoints.js

Run it like:

```sh
node ImportCheckpoints.js
```

Running in Development Mode

You can pass the --dev flag to reduce the retry delay from the default 15000ms (15s) to 6000ms (6s).
This makes iteration faster when testing locally.

```sh
node ImportCheckpoints.js --dev
```

### 4. (Optional) Check That On-chain Checkpoints Match Expected

Modify and re-run ExportCheckpoints.js to extract the checkpoints we just inserted into checkpoints2.json.
Then check that the contents of checkpoints.json exactly match those of checkpoints2.json

Change the file written to on this line to `checkpoints2.json`:
```js
fs.writeFileSync('checkpoints.json', JSON.stringify(sortedCheckpoints, null, 2));
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
diff checkpoints.json checkpoints2.json
```
On success no file differences will be printed

### 5. Attest On Top of Imported Checkpoints

No special steps here. Simply register and run attestors as normal for the chain_key you
imported checkpoints to.

The resulting attestor sync process should take a much shorter time than it otherwise would.

## Importing Checkpoints from a CSV File

`ImportCheckpointsFromCsv.js` is an alternative to the export-then-import workflow above.
Use it when you already have checkpoints in a CSV file (e.g. produced by a checkpoint-builder
tool) and want to import them directly into a chain without needing a live source chain to
export from.

### CSV Format

Each row is `block_number,digest_hex`. An optional header line is supported and will be
skipped automatically.

```
block_number,digest_hex
1000,0x6cfd0730ec3bd606df4a125c004105ba21dbf06a004464e102de156d992ef04b
2000,0xd63f51622a13d598a341266e72a253556ed1f681495006a47b739404c3ea6a90
```

### Configuration

Configuration can be provided via CLI arguments or environment variables. CLI arguments take
priority over environment variables.

| CLI argument  | Env variable              | Description                                        |
|---------------|---------------------------|----------------------------------------------------|
| `--file`      | `CHECKPOINTS_FILE`        | Path to the CSV file                               |
| `--rpc`       | `DESTINATION_CHAIN`       | WebSocket URL of the target chain                  |
| `--chain-key` | `CHAIN_KEY_ON_DESTINATION`| Chain key to import checkpoints into               |
| *(none)*      | `MNEMONIC`                | Sudo account mnemonic (required, env var only)     |

### Usage

Via CLI arguments:

```sh
node ImportCheckpointsFromCsv.js --file checkpoints.csv --rpc ws://127.0.0.1:9944 --chain-key 2
```

Via `.env` file — add to `attestor/scripts/.env`:

```
MNEMONIC="//Alice"
DESTINATION_CHAIN="ws://127.0.0.1:9944"
CHAIN_KEY_ON_DESTINATION=2
CHECKPOINTS_FILE=checkpoints.csv
```

Then run:

```sh
node ImportCheckpointsFromCsv.js
```

Pass `--dev` to reduce the retry delay from 15 s to 6 s, useful when testing locally:

```sh
node ImportCheckpointsFromCsv.js --dev
```

Checkpoints are submitted in batches of 100 via a `sudo(attestation.importCheckpoints(...))` call.
