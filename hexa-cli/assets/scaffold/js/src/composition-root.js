/**
 * {{name}}: a hexagonal skeleton that runs.
 *
 * This is the composition root: the only file allowed to name a concrete
 * adapter. Everything else depends on the port.
 *
 *     domain  <-  ports  <-  usecases
 *                  ^
 *             adapters (secondary)
 *                  ^
 *      composition-root.js: wires them, once
 *
 * Check it with `hexa analyze .`.
 */
import { InMemoryCounterStore } from './adapters/secondary/in-memory-counter-store.js';
import { increment } from './core/usecases/increment.js';

/**
 * Build the application with its real adapters.
 *
 * The one line below is the whole composition decision. Point it at a
 * file-backed store and nothing else moves.
 *
 * @returns {import('./core/ports/counter-store.js').CounterStore}
 */
export function counter() {
  return new InMemoryCounterStore();
}

/** Run the use case against a freshly composed application. */
export function incrementOnce() {
  return increment(counter());
}
