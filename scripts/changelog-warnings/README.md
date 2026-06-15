# Changelog Operator Warnings

A TypeScript script that analyzes git diffs between two release tags and appends operator warnings to generated changelogs. It detects changes in specific areas of the codebase and alerts node, attestor, and indexer operators when action may be required.

## How it works

1. Runs `git diff --name-only <previous-tag>..HEAD` to get the list of changed files
2. Evaluates each rule in `warning-rules.yaml` by matching changed files against configurable include/exclude glob patterns
3. For special cases (like schema changes), inspects the actual diff content to verify meaningful changes occurred
4. Appends a formatted **Operator Warnings** section to the changelog, sorted by severity (critical first)

## Warning rules

Rules are defined in [`warning-rules.yaml`](./warning-rules.yaml). Each rule has:

| Field | Description |
|---|---|
| `id` | Unique identifier |
| `label` | Short title shown in the changelog |
| `description` | Detailed explanation for operators |
| `severity` | `warning` or `critical` (affects sorting and icon) |
| `include` | Glob patterns for files that trigger this rule |
| `exclude` | *(optional)* Glob patterns for files to ignore |
| `contentCheck` | *(optional)* Additional content-level analysis (see [Content checks](#content-checks)) |

### Current rules

| Rule | What triggers it | Severity |
|---|---|---|
| **Node binary changed** | `node/src/**` changed (excluding `chain_spec.rs`) | warning |
| **Pallet/precompile changed** | `pallets/*/src/**` or `precompiles/*/src/**` changed (excluding test files) | warning |
| **Runtime definition changed** | `runtime/src/**` changed (excluding `version.rs`) | critical |
| **Attestor source changed** | Attestor source, primitives, or related common packages changed | warning |
| **Schema entities added/removed** | `cc3-indexer/schema.graphql` has entire entity types added or removed | warning |
| **Schema entities modified** | `cc3-indexer/schema.graphql` has fields added/removed on existing entities (resync from block 0 required) | critical |

To add a new rule, add an entry to `warning-rules.yaml` — no code changes needed unless a new `contentCheck` type is required.

### Content checks

Content checks go beyond file-level detection and inspect the actual diff content. They are configured via the `contentCheck` field on a rule. Available types:

| Type | Description |
|---|---|
| `graphql-entity-added-removed` | Triggers when entire GraphQL entity types (`type Foo @entity { ... }`) are added or removed |
| `graphql-entity-modified` | Triggers when fields are added or removed on **existing** entities (ignores fields inside wholly new/removed entities) |

Content check implementations live in `src/content-checkers/`. To add a new type, create a new file there, export it from `index.ts`, and wire it up in `rules.ts`.

## Usage

```bash
# Install dependencies
npm ci

# Dry run — prints warnings to stdout without modifying any file
npx tsx src/index.ts --previous-tag <tag> --dry-run

# Append warnings to a changelog file
npx tsx src/index.ts --previous-tag <tag> --changelog /path/to/changelog.md

# Use a custom rules file
npx tsx src/index.ts --previous-tag <tag> --dry-run --rules ./custom-rules.yaml
```

### CLI arguments

| Argument | Required | Description |
|---|---|---|
| `--previous-tag <tag>` | Yes | Git tag to diff against (e.g. `3.99.0-devnet`) |
| `--changelog <path>` | Yes* | Path to the changelog file to append warnings to |
| `--dry-run` | No | Print output to stdout instead of modifying the changelog |
| `--rules <path>` | No | Path to a custom rules YAML file (defaults to `warning-rules.yaml`) |

\* Not required when using `--dry-run`.

## CI integration

This script runs automatically as part of the release pipeline in `.github/workflows/release.yml`, inside the `create-release` job. It appends warnings to the generated changelog before the GitHub release is created.

## Sample run

Below is a sample of how the output looks like:

```console
$ npx tsx src/index.ts --previous-tag 3.99.0-devnet --dry-run

Loading rules from: /home/gluwa/Repos/creditcoin3-next/scripts/changelog-warnings/warning-rules.yaml
Loaded 6 warning rules
Getting changed files since 3.99.0-devnet...
Found 132 changed files

3 warning(s) triggered:
  [WARNING] Pallet or precompile code changed (9 files)
  [WARNING] Attestor source changed (38 files)
  [WARNING] Indexer schema entities added or removed (1 files)
```

The CHANGELOG then would get the following appended to it:

## ⚠️ Operator Warnings

> **The following changes in this release may require action from node, attestor, or indexer operators.**
> **Please review carefully before upgrading.**

### 🟡 Pallet or precompile code changed

Pallet or precompile source code has changed. A runtime upgrade
may be required. Review the changes carefully before upgrading.

<details>
<summary>Changed files (9)</summary>

- `pallets/attestation/src/benchmarking.rs`
- `pallets/attestation/src/clear_or_revert.rs`
- `pallets/attestation/src/continuity.rs`
- `pallets/attestation/src/impls.rs`
- `pallets/attestation/src/lib.rs`
- `pallets/attestation/src/weights.rs`
- `pallets/randomness/src/weights.rs`
- `pallets/supported-chains/src/lib.rs`
- `pallets/supported-chains/src/weights.rs`

</details>

### 🟡 Attestor source changed

Attestor-related source code has changed. Attestor operators
should rebuild and redeploy their attestor binary.

<details>
<summary>Changed files (38)</summary>

- `attestor/attestor/src/attestation.rs`
- `attestor/attestor/src/common/mod.rs`
- `attestor/attestor/src/events.rs`
- `attestor/attestor/src/lib.rs`
- `attestor/attestor/src/main.rs`
- `attestor/attestor/src/stream/attestation/error.rs`
- `attestor/attestor/src/stream/attestation/mod.rs`
- `attestor/attestor/src/stream/cc3/error.rs`
- `attestor/attestor/src/stream/cc3/mod.rs`
- `attestor/attestor/src/stream/mod.rs`
- `attestor/attestor/src/worker/api/metrics.rs`
- `attestor/attestor/src/worker/api/mod.rs`
- `attestor/attestor/src/worker/mod.rs`
- `attestor/attestor/src/worker/p2p/mod.rs`
- `attestor/attestor/src/worker/production/mod.rs`
- ... and 23 more

</details>

### 🟡 Indexer schema entities added or removed

The indexer GraphQL schema has had entities added or removed.

<details>
<summary>Changed files (1)</summary>

- `cc3-indexer/schema.graphql`

</details>
