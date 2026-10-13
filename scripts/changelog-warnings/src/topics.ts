import { minimatch } from 'minimatch';

import { ReleaseChange, ReleaseTopicsConfig } from './types.js';

export const DEPENDENCIES = 'dependencies';
export const OTHER = 'other';

function isDependencyBump(change: ReleaseChange, config: ReleaseTopicsConfig): boolean {
    const deps = config.dependencies;
    if (deps.authors?.some((author) => author === change.author || author === change.login)) return true;
    if (!change.type || !deps.types?.includes(change.type)) return false;
    return change.scopes.some((scope) => deps.scopes?.includes(scope));
}

// Picks the one topic a change is listed under. See release-topics.yaml for the order.
export function topicFor(change: ReleaseChange, config: ReleaseTopicsConfig): string {
    if (isDependencyBump(change, config)) return DEPENDENCIES;

    for (const topic of config.topics) {
        if (change.scopes.some((scope) => topic.scopes?.includes(scope))) return topic.id;
    }

    for (const topic of config.topics) {
        if (change.type && topic.types?.includes(change.type)) return topic.id;
    }

    for (const topic of config.topics) {
        const globs = topic.include ?? [];
        if (change.files.some((file) => globs.some((glob) => minimatch(file, glob, { dot: true })))) {
            return topic.id;
        }
    }

    return OTHER;
}

export function isSkipped(change: ReleaseChange, config: ReleaseTopicsConfig): boolean {
    return (config.skip ?? []).some((pattern) => new RegExp(pattern, 'i').test(change.title));
}
