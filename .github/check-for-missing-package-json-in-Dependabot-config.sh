#!/bin/bash

# Check if there are package.json files which have not been specified in
# Dependabot's configuration!
#
# WARNING: needs to be executed from the project root directory

DEPENDABOT_YAML=".github/dependabot.yml"

PACKAGE_JSON_FILES_IN_DEPENDABOT_YAML=$(grep package-ecosystem -A1 "$DEPENDABOT_YAML" | grep -A1 npm | grep directory | cut -f2 -d'"' | sort | while IFS= read -r DIR; do echo ".$DIR/package.json" | tr -s "//"; done)
echo "INFO: package.json files found in $DEPENDABOT_YAML"
echo "$PACKAGE_JSON_FILES_IN_DEPENDABOT_YAML"
echo "----- END -----"
echo
echo

PACKAGE_JSON_FILES_IN_SOURCE_CODE=$(find ./ -name package.json | grep -v node_modules | sort)
echo "INFO: package.json files found in source code"
echo "$PACKAGE_JSON_FILES_IN_SOURCE_CODE"
echo "----- END -----"
echo
echo

MISSING_FILES=0
for FILE in $PACKAGE_JSON_FILES_IN_SOURCE_CODE; do
    if [[ $PACKAGE_JSON_FILES_IN_DEPENDABOT_YAML = *$FILE* ]]; then
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
