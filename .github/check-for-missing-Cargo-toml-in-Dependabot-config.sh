#!/bin/bash

# Check if there are Cargo.toml files which have not been specified in
# Dependabot's configuration!
#
# WARNING: needs to be executed from the project root directory

DEPENDABOT_YAML=".github/dependabot.yml"

CARGO_FILES_IN_SOURCE_CODE=$(find ./ -name Cargo.toml | grep -v "./target/" | grep -v "/node_modules/" | sort)
echo "INFO: Cargo.toml files found in source code"
echo "$CARGO_FILES_IN_SOURCE_CODE"
echo "----- END -----"
echo

MISSING_FILES=0
for FILE in $CARGO_FILES_IN_SOURCE_CODE; do
    PARENT_DIR=$(dirname "$FILE" | sed "s|^\.|/|" | tr -s "/")
    if grep -q "\- \"$PARENT_DIR\"" "$DEPENDABOT_YAML"
    then
        echo "PASS: $FILE is accounted for in $DEPENDABOT_YAML"
    else
        echo "FAIL: $FILE is NOT accounted for in $DEPENDABOT_YAML"
        MISSING_FILES=$((MISSING_FILES + 1))
    fi
done

if [ "$MISSING_FILES" -gt 0 ]; then
    echo "FAIL: There are Cargo.toml files MISSING in $DEPENDABOT_YAML"
else
    echo "PASS: All Cargo.toml files are specified in $DEPENDABOT_YAML"
fi

exit $MISSING_FILES
