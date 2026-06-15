/**
 * Detects whether a GraphQL schema diff contains field-level additions or removals.
 *
 * This uses regex-based parsing rather than a full GraphQL parser, which is
 * sufficient for the consistent SubQuery entity format used in this project.
 *
 * A "field line" is a diff line (starting with + or -) that matches the
 * GraphQL field pattern: `fieldName: Type` with optional modifiers like `!`,
 * `@index`, `@derivedFrom`, etc.
 */

interface SchemaFieldChange {
    added: string[];
    removed: string[];
}

// Matches a GraphQL type declaration like:
//   type EntityName {
//     ...
//   }
const TYPE_PATTERN = /^\s*type\s+\w+/;

const TYPE_END_PATTERN = /^\s*}\s*$/;

// Matches a GraphQL field declaration like:
//   fieldName: Type!
//   fieldName: Type! @index
//   fieldName: [Type] @derivedFrom(field: "foo")
const FIELD_PATTERN = /^\s*(\w+):\s+.+/;

// Lines that look like fields but are actually type/entity declarations or annotations
const IGNORE_PATTERNS = [/^\s*type\s+/, /^\s*#/, /^\s*"""/, /^\s*@/];

function isEntityLine(line: string): boolean {
    return TYPE_PATTERN.test(line);
}

function isTypeEndLine(line: string): boolean {
    return TYPE_END_PATTERN.test(line);
}

function isFieldLine(line: string): boolean {
    if (IGNORE_PATTERNS.some((p) => p.test(line))) {
        return false;
    }

    return FIELD_PATTERN.test(line);
}

export function detectSchemaFieldChanges(diffOutput: string): SchemaFieldChange {
    const lines = diffOutput.split('\n');
    const added: string[] = [];
    const removed: string[] = [];

    // We use a simple state machine to track whether we're currently within an entity/type definition.
    // This helps avoid false positives from field-like lines that are actually added or removed types/entities.
    let entityAddedContext = false;
    let entityRemovedContext = false;

    for (const line of lines) {
        // Skip diff headers
        if (line.startsWith('+++') || line.startsWith('---')) {
            continue;
        }

        const isGitAddition = line.startsWith('+');
        const isGitRemoval = line.startsWith('-');

        // Only process lines that are additions or removals
        if (!isGitAddition && !isGitRemoval) {
            continue;
        }

        const lineContent = line.slice(1).trim();

        // We check for entity/type declarations to update our context,
        // and only consider field lines that are outside of added/removed entity contexts.
        if (isEntityLine(lineContent)) {
            entityAddedContext = isGitAddition;
            entityRemovedContext = isGitRemoval;
        } else if (isGitAddition) {
            // If we encounter a type end line, we reset the context flags
            if (isTypeEndLine(lineContent)) {
                entityAddedContext = false;
            } else if (isFieldLine(lineContent) && !entityAddedContext) {
                // Only consider this an added field if we're not currently in the context of an added entity/type
                added.push(lineContent);
            }
        } else if (isGitRemoval) {
            // If we encounter a type end line, we reset the context flags
            if (isTypeEndLine(lineContent)) {
                entityRemovedContext = false;
            } else if (isFieldLine(lineContent) && !entityRemovedContext) {
                // Only consider this a removed field if we're not currently in the context of a removed entity/type
                removed.push(lineContent);
            }
        }
    }

    return { added, removed };
}

export function hasEntityAddedOrRemoved(diffOutput: string): boolean {
    const lines = diffOutput.split('\n');

    for (const line of lines) {
        // Skip diff headers
        if (line.startsWith('+++') || line.startsWith('---')) {
            continue;
        }

        const isGitAddition = line.startsWith('+');
        const isGitRemoval = line.startsWith('-');

        const lineContent = line.slice(1).trim();

        if (isGitAddition && TYPE_PATTERN.test(lineContent)) {
            return true; // Entity added
        } else if (isGitRemoval && TYPE_PATTERN.test(lineContent)) {
            return true; // Entity removed
        }
    }

    return false;
}

export function hasSchemaFieldChanges(diffOutput: string): boolean {
    const changes = detectSchemaFieldChanges(diffOutput);
    return changes.added.length > 0 || changes.removed.length > 0;
}
