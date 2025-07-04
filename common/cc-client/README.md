# Creditcoin rust client

## Generate artifact

Make sure to rebuild Creditcoin3 first, and then run the node


Now install the `subxt` CLI tool if you haven't done so already:

```bash
cargo install subxt-cli
```

Then, generate the metadata artifact:

```bash
subxt metadata -f bytes > artifacts/metadata.scale
```

## Compile the client

```bash
cargo build
```
