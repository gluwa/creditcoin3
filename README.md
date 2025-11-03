# Creditcoin3

A Creditcoin3 node with the Ethereum RPC support, ready for deploying smart contracts.

## Major components

The following directories correspond to major components:

- `attestor/` and `attestor_zombienet/`
- `cc3-indexer/`
- `cli/`
- `pallets/` and `precompiles/`
- `common/eth/contracts/`
- `query-cli/`

See individual README files for more information


## Supported operating system

The only supported OS is Linux / x86_64 - see the `runs-on:` sections in
`.github/workflows/ci.yml` for the actual distro/version used during testing.

**WARNING:** this repository makes heavy use of symbolic links to account for
inter-dependencies between various components and to avoid artifacts in different
directories diverging from one another. This works well on Linux and MacOS, however
symbolic links are not supported on Windows!  If you see a symlink file being removed
by git and replaced by its content that is most likely the reason.

This is a mistake and should be corrected before merging!


## Dev environment setup

To install tools & binaries used during development execute:

```bash
cargo install subxt-cli --locked
cargo install taplo-cli --locked
```

To install git hooks, which will stop you from committing common mistakes,
from the root directory of this repository execute:

```bash
ln -s ../../.github/hooks/pre-commit .git/hooks/pre-commit
ln -s ../../.github/hooks/pre-push .git/hooks/pre-push
```

## Build & Run

To build the chain, execute the following commands from the project root:

```bash
cargo build --release
```

To execute the chain, run:

```bash
./target/release/creditcoin3-node --dev
```

_WARNING: running natively on Windows [is unsupported](https://github.com/gluwa/creditcoin/security/advisories/GHSA-cx5c-xwcv-vhmq)._

The node also supports to use manual seal (to produce block manually through RPC).
This is also used by the ts-tests:

```bash
$ ./target/release/creditcoin3-node --dev --sealing=manual
# Or
$ ./target/release/creditcoin3-node --dev --sealing=instant
```

## Recommended development workflow

To minimize back-and-forth on pull requests it is recommended that developers would execute
a number of checks locally before pushing a PR:

1. `cargo fmt` and `cargo test` and `cargo clippy` when working on any component written in Rust
2. `yarn format` and `yarn lint` and `yarn typecheck` when working on any component written in TypeScript
3. Execute the primary test(s) for the affected component - see individual README files for more information


It is also advisable that pull requests be:

1. relatively small and related to a single feature / change request so they are easier to review
2. up-to-date aka rebased onto latest development branch
3. not contain "Merge" commits


### Docker Based Development

Optionally, you can build and run creditcoin3-node within Docker directly.
The Dockerfile is optimized for development speed.
(Running the `docker run...` command will recompile the binaries but not the dependencies)

Building (takes 5-10 min):

```bash
docker build -t creditcoin3-node-dev .
```

Running (takes 1 min to rebuild binaries):

```bash
docker run -t creditcoin3-node-dev
```

**WARNING:** when running multiple components in containers, especially when some of them
may be running directly on the host OS make sure that you have your networking setup configured
correctly! `ws://localhost:9944` is interpreted differently when a process executes inside a container!


## Testing

Creditcoin 3 is comprised of multiple components and therefore contains multiple checks and tests.
They can be divided into 3 main categories:

1. Sanity checks - usually quick checks which try to prevent common mistakes. For example:

   - finding duplicate files
   - finding broken symbolic links
   - enforcing code formatting and git sanity rules
   - static analysis

2. Stand alone tests - usually do not require the majority of the other components to be running/present
   and/or places the focus on a single component. For example:

   - unit tests (either Rust or TypeScript)
   - component build jobs

3. Integration & end-to-end testing - require multiple/all of existing components to be running in a
   specific order and/or specific relationship with one another before being able to exercise a component
   and assert on conditions against it. For example:

   - integration tests against the blockchain (extrinsics, precompiles, etc)
   - integration tests against attestation network
   - integration tests against cc3-indexer

Every individual category may contain one or more test suites comprising of many test files and test scenarios
inside those files. We try to select directory structure, file names and scenario names
which are intuitive and self evident as to which the Target Under Test is and what the
actual expected behaviour should be. That could be an extrinsic,
a function name, a specific condition + expected outcome or a functional area for example (e.g Prover.sol).

The rule of thumb for naming and directory structure usually is:

- explicit > implicit
- more explicit is better
- longer > shorter
- more verbose is better


### How to help yourself with various failures in CI jobs

See documentation & screenshots at
https://gluwa.atlassian.net/wiki/spaces/CB/pages/1699119122/How+to+help+yourself+with+various+failures+in+CI+jobs

**NOTE:** most all test jobs produce logs both from executing the test suite itself, reported on the console,
this is what we see in the GitHub interface, as well as logs from running various components in the background.
At the end of execution these are uploaded as artifacts for later use. Can be found under the
`Uploading logs` step or on the summary page.


### Running locally

#### Understanding entry-points

There are 2 primary entry-points one should be concerned with:

1. Entry point to a particular CI job:

   - these are defined in YAML files under `.github/workflows/`. The main one is `ci.yml`
   - Individual CI jobs are defined under the `jobs:` section. Their entry-point is the first
     items under the `steps:` section


2. Entry point to a particular test suite within that CI job - this is usually the command
   which triggers execution of said test suite. Could be

   - `cargo <command>`
   - `docker <command>`
   - a shell script or a command
   - `yarn|npm <command>`

For most integration test jobs that is something like `yarn test` and the actual command + arguments
which is executed can be found inside `package.json` in the respective directory. For example
`yarn test:cc3-indexer` is defined in `cli/package.json` as
`jest --config src/test/cc3-indexer-tests.config.ts --verbose --runInBand --forceExit src/test/cc3-indexer-tests`.

**WARNING:** high-level entry points are unlikely to change, however the exact commands and/or arguments
given to them may change from time to time.

#### Understanding CI job / test suite setup

All CI jobs / test suites require some form of a setup. When it comes to integration testing suites
they need a local chain running, perhaps a few other components too. The relations between change all
the time and is represented by the individual steps in `ci.yml`.

**WARNING:** one needs to execute all setup steps in the order they are defined with the exact same
arguments given in `ci.yml` before being able to execute a specific test suite. Sometimes there are multiple
inter-dependencies between some of these steps and skipping them may lead to unexpected results.

It is always best to copy the order and exact commands from `ci.yml` or use tools like
[act](https://github.com/nektos/act) which operate on the YAML file directly.

#### Running a single test file

For most integration test suites the test runner of choise is [Jest](https://jestjs.io/). In order to execute
a single `.test.ts` file instead of all files inside the directory just pass the name of the file to the
entry-point command. For example:

```bash
yarn jest --config src/test/blockchain-tests.config.ts --silent --verbose --runInBand --forceExit src/test/blockchain-tests/pallets/attestation/attest.test.ts
```

#### Running a single test scenario / group of scenarios

Append `.only` after the respective `.describe` or `.it` function call. For example:

```JavaScript
    it.only('fee is min 0.01 CTC', async (): Promise<void> => {
```

For more detailed documentation see https://jestjs.io/docs/cli.

## Potential issues and how to solve them

In case you find issues during compilation and/or running the project, check below.

### Compilation issues for rocksdb

If you have trouble compiling rocksdb and your system's `clang` version is 20 or higher, try this before compiling:

```sh
export CXXFLAGS="$CXXFLAGS -include cstdint"
```
