# Prover poc

This document explains the prover module

## Pre-requisites

- Running creditcoin3-next chain (Run with `--dev` flag)
- Running eth node (local or remote), for ease of use configure it to have chain id 31337 since this is a test chain that is configured for a prover with this key (see below).
- Attestation network (see [attestator](../attestor/README.md) for more details)
- Docker compose

## Installation (Optional if not running dev mode)

Setup cairo env:

```sh
python3.10 -m venv ~/cairo_venv
pip install -r requirements.txt
```

Run docker compose:

```sh
docker compose up -d
```

## Running

```sh
cargo run -- -v --cc3-key "//Alice" --eth-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133"
```

Check prover attestation cache:

```sh
psql postgres://prover:prover@localhost/attestations
select * from signedattestation;
```

## Dev mode

If you want to run the prover in dev mode, you can use the following command:

```sh
cargo run -- -v --cc3-key "//Alice" --eth-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133" --dev
```

This disables the Cairo prover and uses a dummy proof output instead.

## Submitting a query

See [query-cli](../query-cli/README.md) for more details.

## Claims

Claims that are proven are stored as JSON in `claims` folder.
