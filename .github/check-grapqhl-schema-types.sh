#!/bin/bash

# Check data types in cc3-indexer/schema.graphql!
#
# For example, numerical values should be BitInt! for the most part and
# an actual Int! is the exception rather than the norm.

SCHEMA_FILE="cc3-indexer/schema.graphql"
if [ ! -f "$SCHEMA_FILE" ]; then
    echo "ERROR: file $SCHEMA_FILE does not exist!"
    exit 1
fi

INT_VARIABLES=$(grep "Int!" "$SCHEMA_FILE" | grep -v "BigInt!" | grep -v "this-is-expected")
if [ -n "$INT_VARIABLES" ]; then
    echo "INFO: Variables of type Int! declared inside $SCHEMA_FILE"
    echo "$INT_VARIABLES"

    echo "FAIL: $SCHEMA_FILE declares variables of type Int! which should probably be BigInt!"
    exit 2
fi

echo "PASS: All Int! variables in $SCHEMA_FILE were accounted for"
exit 0
