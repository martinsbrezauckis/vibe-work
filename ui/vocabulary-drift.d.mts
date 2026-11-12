/** Types for the Node vocabulary checker; consumed by its TypeScript contract tests. */
/** Finds conversation wording in TypeScript or JSX source. */
export function chatWording(path: string, source: string): string[];
/** Checks conversation wording in the UI source tree. */
export function checkChatWording(): string[];
/** Checks obsolete tab names in the supplied paths or the default source/doc roots. */
export function checkTabWording(paths?: string[]): string[];
