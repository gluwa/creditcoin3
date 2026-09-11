/**
 * Flush pending store writes to the database before field-filtered reads or deletions.
 *
 * SubQuery buffers entity `.save()` calls in an in-memory store cache and only
 * persists them to the DB between blocks. Read APIs such as `getByFields` resolve
 * against persisted data, so an entity created earlier *in the same block* is not
 * visible to a subsequent field-filtered query and would be silently missed
 * (`get` by id reads through the cache and does not need this).
 *
 * `flush()` lives on the store cache in `@subql/node` but is not declared on the
 * public `@subql/types` `Store` interface, so it is accessed via a guarded cast
 * and no-ops if unavailable (never breaks indexing).
 */
export async function flushStore(): Promise<void> {
    const maybeFlush = (store as unknown as { flush?: () => Promise<void> }).flush;
    if (typeof maybeFlush === 'function') {
        await maybeFlush.call(store);
    }
}
