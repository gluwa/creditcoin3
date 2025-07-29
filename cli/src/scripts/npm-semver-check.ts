import validator from 'validator';

if (process.argv.length < 3) {
    console.error('USAGE: npm-semver-check.ts <version-string>');
    process.exit(1);
}

const verString = process.argv[2];

if (validator.isSemVer(verString)) {
    console.log(`PASS: ${verString} is a valid semver string`);
} else {
    console.log(`FAIL: ${verString} is not a valid semver string`);
    process.exit(2);
}
