---
name: conventional-commits
description: Conventional Commits (commitizen style) for creditcoin3. Use whenever you write or amend a commit message, create or rename a branch for a PR, open a PR, edit a PR title, or are asked to check or clean up commit and PR naming. PRs into usc-dev are squash-merged, so the PR title becomes the commit on usc-dev and must follow this format too.
---

# Conventional Commits for creditcoin3

Every commit message and every PR title in this repo follows
[Conventional Commits](https://www.conventionalcommits.org/) as commitizen checks it.

## Format

```
<type>(<scope>): <subject>

<body>

<footer>
```

- **Header** (`<type>(<scope>): <subject>`) is required: aim for 72 characters, hard limit 100.
- **Body** is optional. Explain *why* the change is needed and what it does not do, wrapped at 72.
- **Footer** is optional: `BREAKING CHANGE: <what breaks and how to migrate>`, `Refs: CSUB-2054`.

### Type

| type | use for |
|---|---|
| `feat` | a new capability for users, operators or integrators |
| `fix` | a bug fix, including audit findings |
| `perf` | a change whose point is speed or resource use |
| `refactor` | restructuring with no behaviour change |
| `test` | tests only |
| `docs` | documentation only |
| `build` | Cargo/npm dependencies, Dockerfiles, toolchain (Dependabot uses `build(deps)`) |
| `ci` | `.github/workflows`, CI scripts, runners |
| `chore` | anything else that ships no behaviour: releases, version bumps, metadata refreshes |
| `revert` | reverting an earlier commit; the body names the reverted SHA |
| `style` | formatting only (rare here: `cargo fmt` runs in CI) |

A breaking change adds `!` after the type or scope (`feat(attestation)!: …`) **and** a
`BREAKING CHANGE:` footer. In this repo that means: storage layout without a migration, a changed
extrinsic, event or error index, a precompile ABI change, or anything that forces attestors to
upgrade in lockstep with the runtime.

### Scope

The scope names the part of the repo that changed, in lowercase: normally the crate or top-level
directory. Recent examples: `archiver`, `attestor`, `attestation`, `proof-gen`, `eth`, `cli`,
`node`, `runtime`, `precompiles`, `continuity`, `stream_eth`, `stream_cc3`, `checkpoint-builder`,
`write-ability`, `hooks`, `docker`, `deps`.

- Two areas: separate with a comma, no space: `fix(eth,continuity): …`.
- Omit the scope rather than invent a vague one (`ci: …` is fine).
- Ticket IDs are **not** scopes. `fix(DO-2338): …` is wrong; see below.

### Subject

- Imperative mood, as in "fix", "add", "keep", not "fixed" or "adds".
- Starts lowercase, no trailing period.
- Says what changes, not which file: `fix(archiver): keep the stream watchdog above the fetch retry budget`,
  not `fix: update main.rs`.
- **Ticket IDs go at the end in parentheses**: `fix(attestation): harden proof of possession (CSUB-2054)`.
  Not `CSUB-2054 (fix) Proof of possession hardening`.
- On a PR title, GitHub appends ` (#1234)` when squash-merging, so the title's hard limit is 92.

### Audit and security fixes

Name the area, not the weakness. The history is public, and a descriptive subject tells an attacker
where to look before the fix is deployed everywhere.

- Good: `fix(attestation): harden commit validation (CSUB-2053)`
- Bad: `fix(attestation): reject attestations with a forged signer set that bypass quorum`

The same applies to the body, the PR description and the branch name. Put the detail in the
ticket.

### No AI attribution

Do not add `Co-Authored-By: Claude …`, `Generated with Claude Code`, session links or any other
tool trailer to commit messages or PR titles.

## Check a message before using it

Run the validator in this skill's directory on every commit header and PR title you write:

```bash
.claude/skills/conventional-commits/check.sh "fix(archiver): keep the watchdog above the retry budget"
.claude/skills/conventional-commits/check.sh --pr-title "feat(eth): raw-RLP block fetch mode"
git log --format=%s origin/usc-dev..HEAD | .claude/skills/conventional-commits/check.sh --stdin
```

It exits non-zero and says why when a header does not conform. `--pr-title` applies the 92-character
limit.

## Committing

1. Pick the type and scope from the diff, not from the branch name.
2. Write the header, run `check.sh` on it, then commit.
3. One logical change per commit. When a fix belongs in an earlier commit on your own branch, use
   `git commit --fixup=<sha>` and `git rebase -i --autosquash`, so the fix doesn't live on as a
   separate "fix review comments" commit.

## Opening a PR

1. Title the PR as the squash commit should read on `usc-dev`, and run `check.sh --pr-title` on it.
2. Base branch is `usc-dev` unless you are doing a release or a hotfix.
3. For PRs into `usc-testnet` or `main`, which are merged with a merge commit, every commit on the
   branch lands as-is, so every commit subject must conform, not only the title.

## An open PR that does not conform

When you create, update or are asked to look at a PR, check its title, and for merge-commit PRs its
commits:

```bash
gh pr view <n> --json title,baseRefName,author --jq '"\(.baseRefName) \(.author.login) \(.title)"'
gh pr view <n> --json commits --jq '.commits[].messageHeadline' \
  | .claude/skills/conventional-commits/check.sh --stdin
```

- **Title wrong on your own PR**: fix it directly, since the title is metadata and changing it
  rewrites nothing.
  `gh pr edit <n> --title "fix(attestation): harden proof of possession (CSUB-2054)"`
- **Title wrong on someone else's PR**: propose the corrected title to the user. Edit it only when
  the user asks, then leave a one-line PR comment saying what changed and why.
- **Commits wrong on a squash-merged PR (into `usc-dev`)**: leave them. Only the title reaches
  `usc-dev`.
- **Commits wrong on a merge-commit PR (into `usc-testnet` or `main`), or wanted clean anyway**:
  rewording rewrites history. Only do it on your own branch, with the user's go-ahead:
  `git rebase -i` with `reword`, then `git push --force-with-lease=<branch>:<old-sha>`. Never
  force-push someone else's branch.

To sweep all open PRs for non-conforming titles:

```bash
gh pr list --state open --limit 100 --json number,title,author \
  --jq '.[] | "\(.number)\t\(.author.login)\t\(.title)"' \
  | while IFS=$'\t' read -r n who title; do
      .claude/skills/conventional-commits/check.sh --pr-title --quiet "$title" \
        || echo "#$n ($who): $title"
    done
```

Report the list with a proposed title for each, and apply only what the user approves.
