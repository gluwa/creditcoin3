#!/bin/sh

if [ "$#" -ne 1 ] && [ "$#" -ne 2 ]; then
  echo "expected arguments: input_path"
  exit 10
fi

INPUT_PATH="./$1"
FORCE=$2

CAIRO_LANG_DIR="../cairo/lang"
STONE_PROVER="../cairo/stone-prover"

PROGRAM_INPUT_FILE="$INPUT_PATH/program_input.json"
AIR_PROVER_CONFIG="$CAIRO_LANG_DIR/cpu_air_prover_config.json"
AIR_PARAMS="$CAIRO_LANG_DIR/cpu_air_params.json"

if [ ! -d "$INPUT_PATH" ]; then
  echo "$INPUT_PATH does not exist. This folder is supposed to hold the 'program_input.json'."
  exit 20
fi

if [ ! -f "$AIR_PARAMS" ]; then
  echo "$AIR_PARAMS does not exist. This file needs to exist so the script can update parameters with accordance to execution trace."
  exit 22
fi

if [ ! -f "$AIR_PROVER_CONFIG" ]; then
  echo "$AIR_PROVER_CONFIG does not exist."
  exit 23
fi

PRIVATE_INPUT="$INPUT_PATH/private_input.json"
PUBLIC_INPUT="$INPUT_PATH/public_input.json"
TRACE_FILE="$INPUT_PATH/trace.json"
#MEMORY_FILE="$INPUT_PATH/memory.json"

TRACE_SIZE=$(stat -c "%s" "$TRACE_FILE")
echo "trace file size: $TRACE_SIZE"
CHAIN_LEN=$(jq '.blocks | length' "$PROGRAM_INPUT_FILE")
echo "chain length from the input file: $CHAIN_LEN"
#PUBLIC_INPUT_SIZE=$(stat -c "%s" "$PUBLIC_INPUT")
#PRIVATE_INPUT_SIZE=$(stat -c "%s" "$PRIVATE_INPUT")

TRACE_SIZE_DIV3=$((TRACE_SIZE / 3))
LOG_TRACE_SIZE_DIV3=$(echo "$TRACE_SIZE_DIV3" | awk '{print log($1)/log(2)}')
FRI_STEPS_SUM=$(jq '.stark.fri.fri_step_list | add' "$AIR_PARAMS")

LOG_LAST_LAYER_DEGREE_BOUND=$((LOG_TRACE_SIZE_DIV3 - FRI_STEPS_SUM))
#LOG_LAST_LAYER_DEGREE_BOUND=$((LOG_TRACE_SIZE_DIV3 - FRI_STEPS_SUM + 4))
LAST_LAYER_DEGREE_BOUND=$((2 << LOG_LAST_LAYER_DEGREE_BOUND))

#log₂(last_layer_degree_bound) + ∑fri_step_list = log₂(#steps) + 4

echo "TRACE_SIZE_DIV3: $TRACE_SIZE_DIV3"
echo "LOG_TRACE_SIZE_DIV3: $LOG_TRACE_SIZE_DIV3"
echo "FRI_STEPS_SUM: $FRI_STEPS_SUM"
echo "LAST_LAYER_DEGREE_BOUND: $LAST_LAYER_DEGREE_BOUND"

touch cpu_air_params_tmp.json
jq --arg LAST_LAYER_DEGREE_BOUND "$LAST_LAYER_DEGREE_BOUND" \
  '.stark.fri.last_layer_degree_bound = ($LAST_LAYER_DEGREE_BOUND | tonumber)' \
  "$AIR_PARAMS" >cpu_air_params_tmp.json && mv cpu_air_params_tmp.json "$AIR_PARAMS"

rm -rf cpu_air_params_tmp.json

PROOF_FILE="$INPUT_PATH/proof.json"
#ATTESTATION_CHAIN_FILE=$INPUT_PATH"/chain_attestation.json"

if [ -f "$PROOF_FILE" ] && [ "$FORCE" != "force" ]; then
  echo "WARNING: $PROOF_FILE already exists, skipping stone-proving."
  exit 43
else
  echo "out_file: $PROOF_FILE"
  echo "private_input_file: $PRIVATE_INPUT"
  echo "public_input_file: $PUBLIC_INPUT"
  echo "prover_config_file: $AIR_PROVER_CONFIG"
  echo "parameter_file: $AIR_PARAMS"
  echo "generating proof (will take a while)..."

  /usr/bin/time -f "%e" "$STONE_PROVER/cpu_air_prover" \
    --out_file="$PROOF_FILE" \
    --private_input_file="$PRIVATE_INPUT" \
    --public_input_file="$PUBLIC_INPUT" \
    --prover_config_file="$AIR_PROVER_CONFIG" \
    --parameter_file="$AIR_PARAMS" \
    -generate_annotations \
    >/dev/null 2>/tmp/elapsed.txt
  if ! "$STONE_PROVER/cpu_air_prover"; then
    message=$(cat /tmp/elapsed.txt)
    echo "cpu_air_prover failed: $message"
    exit 44
  fi
  ELAPSED=$(cat /tmp/elapsed.txt)

  echo "proof generated. Elapsed: $ELAPSED s"
fi
