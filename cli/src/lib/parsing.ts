// Parse valid or exit with error
export const parseHexStringOrExit = parseOrExit(parseHexStringInternal);
export const parseIntegerOrExit = parseOrExit(parseIntegerInternal);
export const parsePercentAsPerbillOrExit = parseOrExit(parsePercentAsPerbillInternal);
export const parseChoiceOrExit = parseChoiceOrExitFn;

// A function that takes a parsing function and returns a new function that does tries to parse or prints the error and exits
function parseOrExit<T>(parse: (input: any) => T): (input: any) => T {
    return (input: any) => {
        try {
            return parse(input);
        } catch (e: any) {
            const error = e as Error;
            console.error(`Unable to parse input. ${error.message}`);
            process.exit(1);
        }
    };
}

function parseChoiceOrExitFn(input: any, choices: string[]): string | never {
    try {
        return parseChoiceInternal(input, choices);
    } catch (e: any) {
        const error = e as Error;
        console.error(`Unable to parse input. ${error.message}`);
        process.exit(1);
    }
}

// Choices must be in Capitalized form: ['Staked', 'Stash']
export function parseChoiceInternal(input: any, choices: string[]): string {
    const choice = input as string;
    const styled = choice.charAt(0).toUpperCase() + choice.slice(1).toLowerCase();
    if (!choices.includes(styled)) {
        throw new Error(`Invalid choice: ${choice}, must be one of ${choices.toString()}`);
    }
    return styled;
}

export function parseBoolean(input: any): boolean {
    return !!input;
}

export function parseIntegerInternal(input: any): number {
    const float = Number.parseFloat(input as string);
    if (float % 1 !== 0) {
        throw new Error('Must be an integer');
    }
    const int = Number.parseInt(input as string, 10);
    return int;
}

export function parseHexStringInternal(input: any): string {
    if (!RegExp(/^0x[\da-f]+$/i).exec(input as string)) {
        throw new Error('Must be a valid hexadecimal number');
    }
    return input as string;
}

export function parsePercentAsPerbillInternal(input: any): number {
    if (RegExp(/[^0-9.]/).exec(input as string)) {
        throw new Error('Percent value must be a number');
    }
    const value = Number.parseFloat(input as string);
    if (value < 0 || value > 100) {
        throw new Error('Percent value must be between 0 and 100');
    }
    return Math.floor(value * 10_000_000);
}

export function inputOrDefault(input: any, defaultValue: string): string {
    if (input === undefined) {
        return defaultValue;
    }
    return input as string;
}

export function requiredInput(input: any, message: string): string {
    if (input === undefined) {
        console.error(message);
        process.exit(1);
    }
    return input as string;
}
