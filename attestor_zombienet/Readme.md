# Attestor zombienet binary

This is a program that spawns attestors in a network. A simply config file controls the number of attestors spawned.

## Configuration

```yaml
default_command: "../target/release/attestor"
# Set run to true if you wanna start the attestor automatically
run: false
default_args:
  - "-v"
  - "--eth-start-block=10"
num_attestors: 6
single_node: false
```

- `default_command`: The attestor binary location
- `run`: Whether to start the attestor automatically (set to false for debugging key funding and other setup)
- `default_args`: The default arguments to pass to the attestor binary
- `num_attestors`: The number of attestors to spawn
- `single_node`: If set to true, it will connect to the default node port (9944) and all attestors will point to that one.

## Running

Single node mode:
```bash
cd ..
cargo build --release
./target/release/attestor_zombienet --cc3-key "//Bob" --config-file attestor_zombienet/config.yaml
```
Multinode mode (specify port ranges for attestors depending on how you want to balance the load):
```bash
cd ..
cargo build --release
./target/release/attestor_zombienet --cc3-key "//Bob" --config-file attestor_zombienet/config.yaml --port-ranges 9944
```

Make sure to have a creditcoin3-next zombienet running and an anvil node. See [attestor docs](../attestor/README.md)

## Increasing committee set size

By default committee set size is set to 3. This is the number of attestors needed in order to reach majority on a voting round to include a new attestation.

To increase this number you can use the polkadotjs UI and connect to one of the nodes. Navigate to extrisnics, select attestor pallet and select the `setCommitteeSetSize` extrinsic. You can set the number of attestors needed to reach majority here.

## Increase max attestors

By default the max number of attestors is set to 100. This is the maximum number of attestors that can be registered at the same time. To increase this number you can use the polkadotjs UI and connect to one of the nodes. Navigate to extrinsics, select attestor pallet and select the `setMaxAttestors` extrinsic. You can set the maximum number of attestors here.

## Running against some other ethereum network than the default (localhost:8545)

Create following config file. The chain must be a supported chain on ccnext

```toml
default_command: "../target/release/attestor"
# Set run to true if you wanna start the attestor automatically
run: true
default_args:
  # - "-v"
  - "--eth-rpc-url=http://localhost:8546"
num_attestors: 5
single_node: true
```

Run the attestor zombienet binary with the config file:

If for example the supported chain key is 3, you can run the attestor zombienet binary like this:

```bash
./target/release/attestor_zombienet --cc3-key "//Bob" --config-file attestor_zombienet/config.yaml --chain-key 3
```

## Automatic key generation, funding and registration

If you wish to run this program to create, fund and register attestors automatically, you need to set the `run` field in the config file to `false`. The program will output the address and keys in a list before it exits.
This feature is particularly useful for setting up a large number of attestors quickly.

```toml
...
run: false
...
```

## Automatic gensesis start block number Configuration

If you want the program to automatically determine the current blocknumber for the source chain and set that as the genesis starting point for the attestor you can provide `--configure-genesis` flag when running the program. This will automatically fetch the current block number from the source chain and set it as the genesis start block number for the attestor.

> !!! You must run with `//Alice` key to be able to set the genesis start block number. Since this is *sudo* call.

```bash
./target/release/attestor_zombienet --cc3-key "//Alice" --configure-genesis
```
