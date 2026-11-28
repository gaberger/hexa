/**
 * Use cases: the application's verbs.
 *
 * Rule 3: `usecases/` imports `domain/` and `ports/` only. It takes the port
 * as a parameter and never chooses which adapter fills it; that choice belongs
 * to the composition root alone.
 */

/**
 * Advance the count by one and return the new value.
 * @param {import('../ports/counter-store.js').CounterStore} store
 * @returns {import('../ports/counter-store.js').Count}
 */
export function increment(store) {
  const next = store.load().next();
  store.save(next);
  return next;
}
