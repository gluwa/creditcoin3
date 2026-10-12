import { execFileSync } from 'child_process';

import { ReleaseChange } from './types.js';

const CONVENTIONAL_TYPES = new Set([
    'feat',
    'fix',
    'perf',
    'refactor',
    'build',
    'ci',
    'docs',
    'chore',
    'test',
    'style',
    'revert',
]);

const PR_SUFFIX = /\s*\(#(\d+)\)\s*$/;
const HEADER = /^([A-Za-z][\w-]*)(?:\(([^)]*)\))?!?:\s*/;

function git(args: string[]): string {
    return execFileSync('git', args, { encoding: 'utf-8', maxBuffer: 50 * 1024 * 1024 });
}

function lines(output: string): string[] {
    return output.split('\n').filter((line) => line.length > 0);
}

export interface ParsedTitle {
    title: string;
    pr?: number;
    type?: string;
    scopes: string[];
}

// Reads a squash-merge subject or PR title: `type(scope, scope)!: subject (#123)`.
// Older titles like `archiver: subject` have no conventional type, so the prefix is
// taken as a scope; titles without a prefix have neither.
export function parseTitle(raw: string): ParsedTitle {
    let title = raw.trim();
    let pr: number | undefined;

    const prMatch = title.match(PR_SUFFIX);
    if (prMatch) {
        pr = Number(prMatch[1]);
        title = title.slice(0, prMatch.index).trim();
    }

    const header = title.match(HEADER);
    if (!header) {
        return { title, pr, scopes: [] };
    }

    const prefix = header[1].toLowerCase();
    const scopes = (header[2] ?? '')
        .split(',')
        .map((scope) => scope.trim().toLowerCase())
        .filter((scope) => scope.length > 0);

    if (CONVENTIONAL_TYPES.has(prefix)) {
        return { title, pr, type: prefix, scopes };
    }

    return { title, pr, scopes: [prefix, ...scopes] };
}

function changedFiles(sha: string): string[] {
    try {
        return lines(git(['diff', '--name-only', `${sha}^1`, sha]));
    } catch {
        // Root commit: nothing to diff against.
        return [];
    }
}

// Commits that are on `head` but not on `previous`, one per change that landed on the
// release branch (`--first-parent`, so a squash or a merge counts once). When `previous`
// is not an ancestor, e.g. the last release came from a hotfix branch, the range reaches
// back to the merge base: anything already released there, matched by PR number or
// subject, is left out.
export function listChanges(previous: string, head: string): ReleaseChange[] {
    const released = new Set<string>();
    for (const subject of lines(git(['log', '--format=%s', `${head}..${previous}`]))) {
        const parsed = parseTitle(subject);
        released.add(parsed.pr !== undefined ? `#${parsed.pr}` : parsed.title);
    }

    const changes: ReleaseChange[] = [];
    for (const line of lines(git(['log', '--first-parent', '--format=%H%x1f%s%x1f%an', `${previous}..${head}`]))) {
        const [sha, subject, author] = line.split('\x1f');
        const parsed = parseTitle(subject);
        if (released.has(parsed.pr !== undefined ? `#${parsed.pr}` : parsed.title)) continue;

        changes.push({ sha, author, ...parsed, files: changedFiles(sha) });
    }

    return changes;
}

interface PullRequest {
    title: string;
    user?: { login?: string };
}

// The squash subject is the PR title at merge time; a title edited afterwards (or a
// cherry-pick) only shows the old text. With a token, take the current PR title and the
// author's GitHub login instead. Any failure keeps what git had.
export async function enrichFromGitHub(changes: ReleaseChange[], repository: string, token: string): Promise<void> {
    for (const change of changes) {
        if (change.pr === undefined) continue;

        try {
            const response = await fetch(`https://api.github.com/repos/${repository}/pulls/${change.pr}`, {
                headers: {
                    Accept: 'application/vnd.github+json',
                    Authorization: `Bearer ${token}`,
                    'X-GitHub-Api-Version': '2022-11-28',
                },
            });
            if (!response.ok) {
                console.warn(`PR #${change.pr}: GitHub answered ${response.status}, keeping the commit subject`);
                continue;
            }

            const pull = (await response.json()) as PullRequest;
            const parsed = parseTitle(pull.title);
            change.title = parsed.title;
            change.type = parsed.type;
            change.scopes = parsed.scopes;
            if (pull.user?.login) change.login = pull.user.login;
        } catch (error) {
            console.warn(`PR #${change.pr}: ${String(error)}, keeping the commit subject`);
        }
    }
}
