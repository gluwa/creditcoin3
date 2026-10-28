#!/bin/bash

# Usage: apply.sh broken-post-upgrade|stalled-mbm
#
# Injects a try-runtime self-test fixture into runtime/src/lib.rs, for
# .github/workflows/try-runtime-selftest.yml. The fixture's `Fixture` type goes at the
# front of the migration tuple it belongs in, and the fixture file is declared as the
# `try_runtime_selftest` module by path, so nothing is copied into the runtime's sources.
#
# The edit is anchored on the impl block and the tuple's opening line, and fails rather
# than guess if either is missing or ambiguous: a fixture that lands nowhere would make
# the check pass for the wrong reason. Never commit the result; `git checkout
# runtime/src/lib.rs` undoes it.

set -euo pipefail

FIXTURE="${1:-}"
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
LIB="$ROOT/runtime/src/lib.rs"
MODULE=try_runtime_selftest

case "$FIXTURE" in
    broken-post-upgrade)
        IMPL='impl frame_system::Config for Runtime {'
        TUPLE='type SingleBlockMigrations = ('
        ;;
    stalled-mbm)
        # Only the non-benchmark `Migrations` is a tuple; under `runtime-benchmarks` it is
        # pallet_migrations' mock, which this build does not enable.
        IMPL='impl pallet_migrations::Config for Runtime {'
        TUPLE='type Migrations = ('
        ;;
    *)
        echo "FAIL: usage: $0 broken-post-upgrade|stalled-mbm"
        exit 1
        ;;
esac

if [ ! -f "$ROOT/.github/try-runtime/fixtures/$FIXTURE.rs" ]; then
    echo "FAIL: no fixture file .github/try-runtime/fixtures/$FIXTURE.rs"
    exit 1
fi
if grep -q "$MODULE" "$LIB"; then
    echo "FAIL: runtime/src/lib.rs already has a self-test fixture"
    exit 1
fi

# Counts the impl blocks and the tuple lines inside them, and inserts into the tuple only
# when there is exactly one of each.
PATCHED=$(mktemp)
trap 'rm -f "$PATCHED"' EXIT
COUNTS=$(awk -v impl="$IMPL" -v tuple="$TUPLE" -v item="crate::$MODULE::Fixture," -v out="$PATCHED" '
    $0 == impl { impls++; inside = 1 }
    inside && $0 == "}" { inside = 0 }
    inside {
        at = index($0, tuple)
        if (at > 0 && substr($0, 1, at - 1) ~ /^[ \t]*$/) {
            tuples++
            end = at + length(tuple)
            rest = substr($0, end)
            $0 = substr($0, 1, end - 1) item (rest == "" ? "" : " ") rest
        }
    }
    { print > out }
    END { print impls + 0, tuples + 0 }
' "$LIB")
read -r IMPLS TUPLES <<<"$COUNTS"
if [ "$IMPLS" != 1 ] || [ "$TUPLES" != 1 ]; then
    echo "FAIL: expected one '$IMPL' block with one '$TUPLE' line in runtime/src/lib.rs, found $IMPLS block(s) and $TUPLES line(s)"
    exit 1
fi

{
    echo
    echo "// try-runtime self-test fixture, injected by .github/try-runtime/fixtures/apply.sh."
    echo "#[path = \"../../.github/try-runtime/fixtures/$FIXTURE.rs\"]"
    echo "mod $MODULE;"
} >>"$PATCHED"
cat "$PATCHED" >"$LIB"

echo "INFO: injected the $FIXTURE fixture into runtime/src/lib.rs"
git -C "$ROOT" --no-pager diff -- runtime/src/lib.rs
