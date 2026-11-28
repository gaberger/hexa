/**
 * Secondary adapters: driven by the application.
 *
 * Rule 4: an adapter imports `ports/` only, never another adapter. Rule 5 is
 * the other half: nothing imports this except the composition root.
 */
import { Count } from '../../core/ports/counter-store.js';

/**
 * A counter store that keeps the count in memory.
 *
 * Swap it for a file or a database by writing another store and changing one
 * line in the composition root. No use case changes.
 *
 * @implements {import('../../core/ports/counter-store.js').CounterStore}
 */
export class InMemoryCounterStore {
  #count = Count.zero();

  load() {
    return this.#count;
  }

  save(count) {
    this.#count = count;
  }
}
