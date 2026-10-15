import { ReleaseChange, ReleaseTopicsConfig } from './types.js';
import { DEPENDENCIES, OTHER } from './topics.js';

export interface ReleaseNotesInput {
    repository: string;
    previousTag: string;
    releaseTag?: string;
    date: string;
    config: ReleaseTopicsConfig;
    byTopic: Map<string, ReleaseChange[]>;
}

function formatChange(change: ReleaseChange, repository: string): string {
    const link =
        change.pr !== undefined
            ? `[#${change.pr}](https://github.com/${repository}/pull/${change.pr})`
            : `[${change.sha.slice(0, 9)}](https://github.com/${repository}/commit/${change.sha})`;
    const author = change.login ? `@${change.login}` : change.author;
    return `- ${change.title} (${link}) ${author}`;
}

function section(label: string, changes: ReleaseChange[], repository: string): string[] {
    return [`### ${label}`, '', ...changes.map((change) => formatChange(change, repository)), ''];
}

export function formatReleaseNotes(input: ReleaseNotesInput): string {
    const { repository, previousTag, releaseTag, date, config, byTopic } = input;
    const head = releaseTag ?? 'HEAD';
    const total = [...byTopic.values()].reduce((sum, changes) => sum + changes.length, 0);

    const lines: string[] = [];
    lines.push(`## ${releaseTag ?? 'Unreleased'} (${date})`);
    lines.push('');
    lines.push(
        `${total} change${total === 1 ? '' : 's'} since ${previousTag} ` +
            `([full diff](https://github.com/${repository}/compare/${previousTag}...${head})).`,
    );
    lines.push('');

    for (const topic of config.topics) {
        const changes = byTopic.get(topic.id);
        if (changes?.length) lines.push(...section(topic.label, changes, repository));
    }

    const other = byTopic.get(OTHER);
    if (other?.length) lines.push(...section('Other', other, repository));

    const deps = byTopic.get(DEPENDENCIES);
    if (deps?.length) {
        lines.push(`<details>`);
        lines.push(`<summary>${config.dependencies.label} (${deps.length})</summary>`);
        lines.push('');
        lines.push(...deps.map((change) => formatChange(change, repository)));
        lines.push('');
        lines.push('</details>');
        lines.push('');
    }

    return lines.join('\n');
}
