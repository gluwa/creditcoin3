# Prover poc

This document explains the prover module

## Pre-requisites

- Running creditcoin3-next chain (Run with `--dev` flag)
- Running eth node (local or remote), for ease of use configure it to have chain id 31337 since this is a test chain that is configured for a prover with this key (see below).
- Docker compose

## Configuration file

The configuration file is a toml file that contains the following fields:

```toml
[[chain]]
rpc_url = "https://example.com"
chain_id = 1
price = 1
```

Chain can be repeated multiple times to add multiple chains.

## Running

Start the side services first (there is an adminer app running on localhost:81):

```sh
docker compose up -d
```

Run diesel migration:

```sh
diesel migration run --database-url=postgres://prover:prover@localhost/attestations
```

Run the prover:

First setup the cairo env:

```sh
python3.10 -m venv ~/cairo_venv
pip install -r requirements.txt
```

Then run the prover:

```sh
cargo run -- -v --cc3-key "involve bridge disagree copy aim auction ready garlic industry flee echo era"  --nickname dylan --config-file ./config.toml
```

Check prover attestation cache:

```sh
psql postgres://prover:prover@localhost/attestations
select * from signedattestation;
```

## Submitting a claim (Via polkadotJS)

1. Go to the polkadotJS extrinsic page
2. Select the `prover` module and `submitClaim` extrinsic
3. Fill in the fields
4. Submit the transaction

See example:

[alt_submit_claim](./assets/submit_claim.png)

## Claims

Claims that are proven are stored as JSON in `claims` folder.
