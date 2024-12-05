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

## Configuring your $PATH

The source code relies on the fact that the operating system will be able to
discover the following external components:

- `cpu_air_prover`
- `cpu_air_verifier`
- `stone_prove_claim.sh`
- `verify_merkle_proof.sh`

In order for this to actually happen you have to adjust the `PATH` environment
variable:

```sh
export PATH="$PATH:<repo-root>/cairo/scripts:<repo-root>/cairo/stone-prover:<repo-root>/cairo/stone-verifier"
```

Replace `<repo-root>` with the path to this repository, usually something like `~/creditcoin3-next`!


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

## External prover example

If you wish you can run the prover in `light` mode (`--light-mode` or `-l`).

In light mode it will not stone proof the query but instead it will compute the inputs files for the stone prover and send them over the network to the external prover.

The most recent prover network prototype makes use of an Azure Data Factory Pipeline. It maintains a queue of proving jobs in relational DB format, stores relevant proving inputs and outputs, and manages dynamic resource scaling to assign only as many provers as we have proving jobs.

Repo Link: https://dev.azure.com/gluwa/Gluwa/_git/CCNext.StoneProver.BE?path=/README.md
Azure resource group: https://portal.azure.com/#@gluwa.com/resource/subscriptions/3d91f14a-f591-496b-a3d1-f198b84caaa3/resourceGroups/minhplayground/overview 

TODO: Finish documentation for running light prover with proving network
