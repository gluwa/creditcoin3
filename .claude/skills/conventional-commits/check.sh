#!/usr/bin/env bash
# Validate commit headers / PR titles against the repo's Conventional Commits rules.
#
#   check.sh "fix(archiver): keep the watchdog above the retry budget"
#   check.sh --pr-title "feat(eth): raw-RLP block fetch mode"
#   check.sh --pr-title --ticket CSUB-2054 "fix(attestation): harden proof of possession (CSUB-2054)"
#   git log --format=%s origin/usc-dev..HEAD | check.sh --stdin
#
# Flags: --pr-title (92-char limit, since GitHub appends " (#1234)" on squash), --stdin (one header
# per line), --quiet (no output, exit status only), --ticket KEY (repeatable: the header must end with
# "(KEY)" / "(KEY, KEY2)"). Exits 1 if any header fails. The hard limit is commitlint's conventional
# default (100); aim for 72 so `git log --oneline` stays readable.
#
# Only keys of the team's Jira projects count as ticket keys (CC3_JIRA_PROJECTS, default "CSUB|DO"),
# so identifiers such as EIP-1559, ERC-20, SHA-256 or UTF-8 are never mistaken for tickets.
set -uo pipefail

types='feat|fix|perf|refactor|test|docs|build|ci|chore|revert|style'
max=100
from_stdin=0
quiet=0
headers=()
tickets=()
jira="(${CC3_JIRA_PROJECTS:-CSUB|DO})-[0-9]+"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --pr-title) max=92 ;;
    --stdin) from_stdin=1 ;;
    --quiet) quiet=1 ;;
    --ticket)
      shift
      if ! [[ "${1:-}" =~ ^$jira$ ]]; then
        echo "--ticket needs a Jira key of a known project (${CC3_JIRA_PROJECTS:-CSUB|DO}), got '${1:-}'" >&2
        exit 2
      fi
      tickets+=("$1") ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) headers+=("$1") ;;
  esac
  shift
done

if [ "$from_stdin" -eq 1 ]; then
  while IFS= read -r line; do headers+=("$line"); done
fi

if [ "${#headers[@]}" -eq 0 ]; then
  # Nothing piped in (e.g. a branch with no commits ahead of its base) is a pass, not misuse.
  [ "$from_stdin" -eq 1 ] && exit 0
  echo "usage: check.sh [--pr-title] [--quiet] [--ticket KEY] <header> | --stdin" >&2
  exit 2
fi

say() { [ "$quiet" -eq 1 ] || printf '%s\n' "$*"; }

failed=0
for header in "${headers[@]}"; do
  # Git's own generated subjects are not ours to police.
  case "$header" in
    fixup!\ *|squash!\ *|amend!\ *) continue ;;
  esac

  problems=()
  if ! [[ "$header" =~ ^($types)(\(([a-z0-9._/-]+(,[a-z0-9._/-]+)*)\))?!?:\ (.+)$ ]]; then
    if [[ "$header" =~ ^$jira ]]; then
      problems+=("starts with a ticket ID; use '<type>(<scope>): <subject> (${BASH_REMATCH[0]})'")
    elif [[ "$header" =~ ^[a-z]+(\([^\)]*\))?!?:\  ]]; then
      scope="${BASH_REMATCH[1]}"
      if [[ -n "$scope" && ! "$scope" =~ ^\([a-z0-9._/-]+(,[a-z0-9._/-]+)*\)$ ]]; then
        problems+=("scope must be lowercase crate/area names, comma-separated, no ticket IDs")
      else
        problems+=("unknown type; use one of: ${types//|/, }")
      fi
    else
      problems+=("not '<type>(<scope>): <subject>'")
    fi
  else
    subject="${BASH_REMATCH[5]}"
    [[ "$subject" =~ ^[A-Z][a-z] ]] && problems+=("subject should start lowercase")
    [[ "$subject" == *. ]] && problems+=("subject should not end with a period")
    [[ "$subject" =~ ^(added|adds|fixed|fixes|updated|updates|removed|removes)\  ]] \
      && problems+=("use the imperative ('add', 'fix', 'update', 'remove')")
  fi
  [ "${#header}" -gt "$max" ] && problems+=("header is ${#header} chars (max $max)")

  # Jira keys belong in one trailing group: "... (CSUB-1)" or "... (CSUB-1, CSUB-2)".
  trailing=""
  if [[ "$header" =~ \(($jira(,\ $jira)*)\)$ ]]; then
    trailing="${BASH_REMATCH[1]}"
  fi
  body_part="${header%"($trailing)"}"
  [ -z "$trailing" ] && body_part="$header"
  if [[ "$body_part" =~ $jira ]]; then
    problems+=("Jira key '${BASH_REMATCH[0]}' belongs at the end in parentheses, e.g. '(${BASH_REMATCH[0]})'")
  fi
  for key in ${tickets[@]+"${tickets[@]}"}; do
    if ! [[ ", $trailing, " == *", $key, "* ]]; then
      problems+=("missing Jira key: end the title with '($key)'")
    fi
  done

  if [ "${#problems[@]}" -gt 0 ]; then
    failed=1
    say "✗ $header"
    for p in "${problems[@]}"; do say "    - $p"; done
  else
    say "✓ $header"
  fi
done

exit "$failed"
