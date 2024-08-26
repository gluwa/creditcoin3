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

```bash
cd ..
cargo build --release
./target/release/attestor_zombienet --cc3-key "//Bob" -c attestor_zombienet/config.yaml
```

Make sure to have a creditcoin3-next zombienet running and an anvil node. See [attestor docs](../attestor/README.md)

## Increasing committee set size

By default committee set size is set to 3. This is the number of attestors needed in order to reach majority on a voting round to include a new attestation.

To increase this number you can use the polkadotjs UI and connect to one of the nodes. Navigate to extrisnics, select attestor pallet and select the `setComitteeSetSize` extrinsic. You can set the number of attestors needed to reach majority here.

## Increase max attestors

By default the max number of attestors is set to 100. This is the maximum number of attestors that can be registered at the same time. To increase this number you can use the polkadotjs UI and connect to one of the nodes. Navigate to extrinsics, select attestor pallet and select the `setMaxAttestors` extrinsic. You can set the maximum number of attestors here.
