import { TriggeredWarning } from './types.js';

const MAX_FILES_SHOWN = 15;

function formatWarning(warning: TriggeredWarning): string {
    const { rule, matchedFiles } = warning;
    const icon = rule.severity === 'critical' ? '🔴' : '🟡';
    const lines: string[] = [];

    lines.push(`### ${icon} ${rule.label}`);
    lines.push('');
    lines.push(rule.description.trim());
    lines.push('');
    lines.push('<details>');
    lines.push(`<summary>Changed files (${matchedFiles.length})</summary>`);
    lines.push('');

    const filesToShow = matchedFiles.slice(0, MAX_FILES_SHOWN);
    for (const file of filesToShow) {
        lines.push(`- \`${file}\``);
    }

    if (matchedFiles.length > MAX_FILES_SHOWN) {
        lines.push(`- ... and ${matchedFiles.length - MAX_FILES_SHOWN} more`);
    }

    lines.push('');
    lines.push('</details>');

    return lines.join('\n');
}

export function formatWarningsSection(warnings: TriggeredWarning[]): string {
    const lines: string[] = [];

    lines.push('');
    lines.push('---');
    lines.push('');
    lines.push('## ⚠️ Operator Warnings');
    lines.push('');
    lines.push(
        '> **The following changes in this release may require action from node, attestor, or indexer operators.**',
    );
    lines.push('> **Please review carefully before upgrading.**');
    lines.push('');

    // Sort critical warnings first
    const sorted = [...warnings].sort((a, b) => {
        if (a.rule.severity === 'critical' && b.rule.severity !== 'critical') return -1;
        if (a.rule.severity !== 'critical' && b.rule.severity === 'critical') return 1;
        return 0;
    });

    for (const warning of sorted) {
        lines.push(formatWarning(warning));
        lines.push('');
    }

    return lines.join('\n');
}
