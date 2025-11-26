# cc3-indexer

## Installation

```
yarn install
```

## Build

```
yarn build
```

## Start

```
yarn start:docker
```

If you are targetting local instance, the indexer will crash with:

```
subquery-node-1   | 2025-06-13T11:26:19.870Z <api> ERROR Failed to init ws://host.docker.internal:9944/: Error: Value of ChainId does not match across all endpoints
subquery-node-1   |
subquery-node-1   |        Expected: 0xaf63a9d9e894c1988c58a4a4d5c6b353a2c427f957e142909a70baac5fd47628
subquery-node-1   |        Actual: 0x36a8d8ddb80c319b819671e5b9d4aa2f0fbc4245cf3b96244156fc6e6e32d93f
subquery-node-1   | 2025-06-13T11:26:19.870Z <nestjs> ERROR undefined Error: All endpoints failed to initialize. Please add healthier endpoints
subquery-node-1   | Cause: AggregateError: All promises were rejected
```

Replace `CHAIN_ID` in `.env` with the chain ID of your local instance, e.g. `0x36a8d8ddb80c319b819671e5b9d4aa2f0fbc4245cf3b96244156fc6e6e32d93f` and rebuild:

```
yarn build
yarn start:docker
```

## Development flow

### Changes to types or schema

If you change types or schema, you need to follow these steps:

1. Update the `schema.graphql` file with your changes.
2. Run `yarn codegen` to generate the TypeScript types from the updated schema.
3. Run `yarn build` to compile the TypeScript code.

### Adding or updating a handler

If you want to add a new handler or update an existing one, follow these steps:

1. Create or update the handler in `datasources.ts` to define how to process the data.
2. Implement the logic for the handler in `src/mappings` to parse and store the data.
3. Add the new handler to `mappingHandlers.ts` to register it with the indexer.

### Changing env

If you change the `.env` file, you need to rebuild the project to apply the changes.

### Reset data

If you want to reset the data in the indexer, you can do so by running:

```
docker compose down
yarn start:docker
```

## Event Handlers

### Native Query Verifier Events

The indexer tracks query verification events from the Native Query Verifier precompile at address `0x0FD2`:

- **TransactionVerified**: Emitted when a transaction is successfully verified
  - Event signature: `TransactionVerified(uint64 indexed chain_key, uint64 indexed height, uint64 transactionIndex)`
  - Stores the chain key, block height, transaction index, and verification metadata
  - This event only fires on successful verification (the precompile reverts on failure)

These events are handled in `src/mappings/evmHandlers.ts` and stored in the `TransactionVerified` entity.

## Testing

The primary CI job for cc3-indexer is `cc3-indexer-testing:` inside `.github/workflows/ci.yml`.
It simulates ingestion of source chain(s) and performs various on-chain actions then
checks whether or not they are represented as expected in cc3-indexer by querying its
GraphQL interface.

The entry-point for the test suite is `test:cc3-indexer` in `cli/package.json` and the primary test suite
location is under `cli/src/test/cc3-indexer-tests/`.
