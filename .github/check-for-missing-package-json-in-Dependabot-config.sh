#!/bin/bash

# Check if there are package.json files which have not been specified in
# Dependabot's configuration!
#
# WARNING: needs to be executed from the project root directory

DEPENDABOT_YAML=".github/dependabot.yml"

PACKAGE_JSON_FILES_IN_SOURCE_CODE=$(find ./ -name package.json | grep -v node_modules | sort)
echo "INFO: package.json files found in source code"
echo "$PACKAGE_JSON_FILES_IN_SOURCE_CODE"
echo "----- END -----"
echo

MISSING_FILES=0
for FILE in $PACKAGE_JSON_FILES_IN_SOURCE_CODE; do
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
    echo "FAIL: There are package.json files MISSING in $DEPENDABOT_YAML"
else
    echo "PASS: All package.json files are specified in $DEPENDABOT_YAML"
fi

exit $MISSING_FILES
