import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';

import { WarningRulesConfig } from './types.js';
import { getChangedFiles } from './git.js';
import { evaluateRules } from './rules.js';
import { formatWarningsSection } from './formatter.js';

function parseArgs(): { previousTag: string; changelogPath: string; dryRun: boolean; rulesPath: string } {
    const args = process.argv.slice(2);
    let previousTag = '';
    let changelogPath = '';
    let dryRun = false;
    let rulesPath = '';

    for (let i = 0; i < args.length; i++) {
        switch (args[i]) {
            case '--previous-tag':
                previousTag = args[++i];
                break;
            case '--changelog':
                changelogPath = args[++i];
                break;
            case '--dry-run':
                dryRun = true;
                break;
            case '--rules':
                rulesPath = args[++i];
                break;
            default:
                console.error(`Unknown argument: ${args[i]}`);
                process.exit(1);
        }
    }

    if (!previousTag) {
        console.error('Error: --previous-tag is required');
        process.exit(1);
    }

    if (!changelogPath && !dryRun) {
        console.error('Error: --changelog is required (or use --dry-run)');
        process.exit(1);
    }

    if (!rulesPath) {
        rulesPath = path.join(__dirname, '..', 'warning-rules.yaml');
    }

    return { previousTag, changelogPath, dryRun, rulesPath };
}

function loadRules(rulesPath: string): WarningRulesConfig {
    if (!fs.existsSync(rulesPath)) {
        console.error(`Error: rules file not found at ${rulesPath}`);
        process.exit(1);
    }

    const content = fs.readFileSync(rulesPath, 'utf-8');
    return yaml.load(content) as WarningRulesConfig;
}

function main() {
    const { previousTag, changelogPath, dryRun, rulesPath } = parseArgs();

    console.log(`Loading rules from: ${rulesPath}`);
    const config = loadRules(rulesPath);
    console.log(`Loaded ${config.rules.length} warning rules`);

    console.log(`Getting changed files since ${previousTag}...`);
    const changedFiles = getChangedFiles(previousTag);
    console.log(`Found ${changedFiles.length} changed files`);

    if (changedFiles.length === 0) {
        console.log('No changed files found. Nothing to do.');
        return;
    }

    const triggeredRules = evaluateRules(config.rules, changedFiles, previousTag);

    if (triggeredRules.length === 0) {
        console.log('No operator warnings triggered.');
        return;
    }

    console.log(`\n${triggeredRules.length} warning(s) triggered:`);
    for (const triggeredWarning of triggeredRules) {
        const icon = triggeredWarning.rule.severity.toUpperCase();
        console.log(`  [${icon}] ${triggeredWarning.rule.label} (${triggeredWarning.matchedFiles.length} files)`);
    }

    const warningsSection = formatWarningsSection(triggeredRules);

    if (dryRun) {
        console.log('\n--- DRY RUN: Would append the following to changelog ---');
        console.log(warningsSection);
        return;
    }

    if (!fs.existsSync(changelogPath)) {
        console.error(`Error: changelog file not found at ${changelogPath}`);
        process.exit(1);
    }

    fs.appendFileSync(changelogPath, warningsSection, 'utf-8');
    console.log(`\nOperator warnings appended to ${changelogPath}`);
}

main();
