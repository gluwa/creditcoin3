#!/bin/sh

# Shell wrapper that delegates to the Python implementation while preserving the original flow

if [ "$#" -ne 1 ] && [ "$#" -ne 2 ]; then
  echo "expected arguments: input_path"
  exit 10
fi

INPUT_PATH="$1"
FORCE="$2"

PARENT_DIR=$(dirname "$0")
CAIRO_ROOT=$(dirname "$PARENT_DIR")
CAIRO_LANG_DIR="$CAIRO_ROOT/lang"
STONE_PROVER_DIR="$CAIRO_ROOT/stone-prover"

# Execute the Python implementation with expected arguments
if [ "$FORCE" = "force" ]; then
  exec python3 "$PARENT_DIR/stone_prove.py" "$INPUT_PATH" "$CAIRO_LANG_DIR" "$STONE_PROVER_DIR" force generate_annotations
else
  exec python3 "$PARENT_DIR/stone_prove.py" "$INPUT_PATH" "$CAIRO_LANG_DIR" "$STONE_PROVER_DIR" generate_annotations
fi
