import { minimatch } from 'minimatch';

import { WarningRule, TriggeredWarning } from './types.js';
import { getFileDiff } from './git.js';
import { hasSchemaFieldChanges, hasEntityAddedOrRemoved } from './content-checkers/index.js';

function fileMatchesPatterns(file: string, patterns: string[]): boolean {
    return patterns.some((pattern) => minimatch(file, pattern));
}

function getMatchingFiles(changedFiles: string[], rule: WarningRule): string[] {
    return changedFiles.filter((file) => {
        const included = fileMatchesPatterns(file, rule.include);
        if (!included) return false;

        if (rule.exclude && rule.exclude.length > 0) {
            return !fileMatchesPatterns(file, rule.exclude);
        }

        return true;
    });
}

function passesContentCheck(rule: WarningRule, previousTag: string): boolean {
    if (!rule.contentCheck) return true;

    if (rule.contentCheck.type === 'graphql-entity-modified') {
        const diff = getFileDiff(previousTag, rule.contentCheck.file);
        if (!diff) return false;
        return hasSchemaFieldChanges(diff);
    } else if (rule.contentCheck.type === 'graphql-entity-added-removed') {
        const diff = getFileDiff(previousTag, rule.contentCheck.file);
        if (!diff) return false;
        return hasEntityAddedOrRemoved(diff);
    } else {
        console.warn(`Unknown content check type: ${rule.contentCheck.type}`);
    }

    return true;
}

export function evaluateRules(rules: WarningRule[], changedFiles: string[], previousTag: string): TriggeredWarning[] {
    const triggered: TriggeredWarning[] = [];

    for (const rule of rules) {
        const matchedFiles = getMatchingFiles(changedFiles, rule);
        if (matchedFiles.length === 0) continue;

        if (!passesContentCheck(rule, previousTag)) continue;

        triggered.push({ rule, matchedFiles });
    }

    return triggered;
}
