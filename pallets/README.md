## Testing

The primary CI jobs for precompiles are `unit-test-creditcoin:` and `integration-test-blockchain:`
inside `.github/workflows/ci.yml`.

The entry-point for the unit test suite is `cargo test` and test functions are usually located
inside `src/lib.rs` and/or `src/tests.rs` files in each sub-directory.

The entry-point for the integration test suite is `test:blockchain` in `cli/package.json`
and the test suite location is under `cli/src/test/blockchain-tests/`.
