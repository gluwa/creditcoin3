import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';

import { ReleaseChange, ReleaseTopicsConfig } from './types.js';
import { enrichFromGitHub, listChanges } from './commits.js';
import { isSkipped, topicFor } from './topics.js';
import { formatReleaseNotes } from './notes-formatter.js';

interface Args {
    previousTag: string;
    releaseTag?: string;
    head: string;
    output?: string;
    topicsPath: string;
}

function parseArgs(): Args {
    const args = process.argv.slice(2);
    let previousTag = '';
    let releaseTag: string | undefined;
    let head = 'HEAD';
    let output: string | undefined;
    let topicsPath = '';

    for (let i = 0; i < args.length; i++) {
        switch (args[i]) {
            case '--previous-tag':
                previousTag = args[++i];
                break;
            case '--tag':
                releaseTag = args[++i];
                break;
            case '--head':
                head = args[++i];
                break;
            case '--output':
                output = args[++i];
                break;
            case '--topics':
                topicsPath = args[++i];
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

    if (!topicsPath) {
        topicsPath = path.join(__dirname, '..', 'release-topics.yaml');
    }

    return { previousTag, releaseTag: releaseTag || undefined, head, output, topicsPath };
}

function loadTopics(topicsPath: string): ReleaseTopicsConfig {
    if (!fs.existsSync(topicsPath)) {
        console.error(`Error: topics file not found at ${topicsPath}`);
        process.exit(1);
    }

    return yaml.load(fs.readFileSync(topicsPath, 'utf-8')) as ReleaseTopicsConfig;
}

async function main() {
    const { previousTag, releaseTag, head, output, topicsPath } = parseArgs();
    const config = loadTopics(topicsPath);
    const repository = process.env.GITHUB_REPOSITORY || 'gluwa/creditcoin3';

    console.log(`Listing changes from ${previousTag} to ${head}...`);
    const changes = listChanges(previousTag, head);
    console.log(`Found ${changes.length} changes`);

    const token = process.env.GITHUB_TOKEN;
    if (token) {
        await enrichFromGitHub(changes, repository, token);
    } else {
        console.log('No GITHUB_TOKEN: using commit subjects and git author names');
    }

    const byTopic = new Map<string, ReleaseChange[]>();
    for (const change of changes) {
        if (isSkipped(change, config)) continue;
        const topic = topicFor(change, config);
        byTopic.set(topic, [...(byTopic.get(topic) ?? []), change]);
    }

    const notes = formatReleaseNotes({
        repository,
        previousTag,
        releaseTag,
        date: new Date().toISOString().slice(0, 10),
        config,
        byTopic,
    });

    if (output) {
        fs.writeFileSync(output, notes, 'utf-8');
        console.log(`Release notes written to ${output}`);
    } else {
        console.log(notes);
    }
}

main().catch((error) => {
    console.error(error);
    process.exit(1);
});
