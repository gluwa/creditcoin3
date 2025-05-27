# cc3-indexer

cc3-indexer

1. add into schema.graphql your database entity to store
1a. If changing public prover contract, then run build.sh in common/eth/contracts
2. yarn codegen
3. yarn build
4. project.ts -> new handler
5. yarn build
6. mappingHandlers.ts -> add code how to parse, and store
7. remove docker running container
8. remove data/postgress folder
9. yarn build
10. yarn start:docker
11. if you change the .env file with a new source of blockchain url you need!!!! remove images to have a new so you would not get an stragne error like docker errors. VERY important!!! remove image!!!

## Testing

The primary CI job for cc3-indexer is `cc3-indexer-testing:` inside `.github/workflows/ci.yml`.
It simulates ingestion of source chain(s) and performs various on-chain actions then
checks whether or not they are represented as expected in cc3-indexer by querying its
GraphQL interface.

The entry-point for the test suite is `test:cc3-indexer` in `cli/package.json` and the primary test suite
location is under `cli/src/test/cc3-indexer-tests/`.
