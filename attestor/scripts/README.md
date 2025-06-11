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

## Import Checkpoints 


After using the `import_checkpoints` sudo call, we need to be careful how we begin 
attesting. the argument `eth_start_block` needs
to be 