import { runSuite } from './run-suite.mjs';

/**
 * Vacuous gate 1 of 3: the runner that matches nothing.
 *
 * Nobody writes this on purpose. It is what is left after a suffix is
 * renamed, a directory moves, or a glob is typed with one character wrong:
 * the runner starts, matches no file, has nothing to fail on, and exits 0.
 * The suite still exists — it is right there in dist/, ending in .test.js —
 * and this gate never touches it.
 */
runSuite('dist', '.spec.js');
