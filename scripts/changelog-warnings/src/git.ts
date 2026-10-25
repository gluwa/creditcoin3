import { execSync } from 'child_process';

export function getChangedFiles(previousTag: string): string[] {
    const output = execSync(`git diff --name-only ${previousTag}..HEAD`, {
        encoding: 'utf-8',
        maxBuffer: 10 * 1024 * 1024,
    });
    return output
        .trim()
        .split('\n')
        .filter((line) => line.length > 0);
}

export function getFileDiff(previousTag: string, filePath: string): string {
    try {
        return execSync(`git diff ${previousTag}..HEAD -- ${filePath}`, {
            encoding: 'utf-8',
            maxBuffer: 10 * 1024 * 1024,
        });
    } catch {
        return '';
    }
}
