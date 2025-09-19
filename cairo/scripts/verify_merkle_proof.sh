#!/bin/sh

# Shell wrapper that delegates to the Python implementation while preserving the original flow

if [ "$#" -ne 1 ] && [ "$#" -ne 2 ]; then
  echo "expected arguments: input_path proof_mode(optional)"
  exit 10
fi

PARENT_DIR=$(dirname "$0")
SOURCE_FILE="$PARENT_DIR/verify_merkle_proof.cairo"
INPUT_PATH="$1"
PROOF_MODE="$2"

# Build args for Python: program_source_path input_path [proof_mode]
if [ -n "$PROOF_MODE" ]; then
  exec python3 "$PARENT_DIR/run_cairo_program.py" "$SOURCE_FILE" "$INPUT_PATH" proof_mode
else
  exec python3 "$PARENT_DIR/run_cairo_program.py" "$SOURCE_FILE" "$INPUT_PATH"
fi
