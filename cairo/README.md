# How to

**WARNING:** `cpu_air_prover` and `cpu_air_verifier` must be in $PATH!
Adjust your machine if necessary!


# Testing

The primary CI jobs for Cairo are `sanity-test-cairo:` and `sanity-test-stone-binaries:`
inside `.github/workflows/ci.yml`. They exercise example programs in order to verify
that this external component isn't completely broken.

More implicit testing of Cairo is exercised as part of various unit and integration tests,
primarily ones which work with valid queries, for example in
`host/src/command.rs` or `cli/src/test/blockchain-tests/pallets/prover/submit-proof.test.ts`.
