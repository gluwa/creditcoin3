#!/usr/bin/env bash
# Validate commit headers / PR titles against the repo's Conventional Commits rules.
#
#   check.sh "fix(archiver): keep the watchdog above the retry budget"
#   check.sh --pr-title "feat(eth): raw-RLP block fetch mode"
#   git log --format=%s origin/usc-dev..HEAD | check.sh --stdin
#
# Flags: --pr-title (92-char limit, since GitHub appends " (#1234)" on squash), --stdin (one header
# per line), --quiet (no output, exit status only). Exits 1 if any header fails. The hard limit is
# commitlint's conventional default (100); aim for 72 so `git log --oneline` stays readable.
set -uo pipefail

types='feat|fix|perf|refactor|test|docs|build|ci|chore|revert|style'
max=100
from_stdin=0
quiet=0
headers=()

for arg in "$@"; do
  case "$arg" in
    --pr-title) max=92 ;;
    --stdin) from_stdin=1 ;;
    --quiet) quiet=1 ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) headers+=("$arg") ;;
  esac
done

if [ "$from_stdin" -eq 1 ]; then
  while IFS= read -r line; do headers+=("$line"); done
fi

if [ "${#headers[@]}" -eq 0 ]; then
  echo "usage: check.sh [--pr-title] [--quiet] <header> | --stdin" >&2
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
    if [[ "$header" =~ ^[A-Z]+-[0-9]+ ]]; then
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

  if [ "${#problems[@]}" -gt 0 ]; then
    failed=1
    say "✗ $header"
    for p in "${problems[@]}"; do say "    - $p"; done
  else
    say "✓ $header"
  fi
done

exit "$failed"
