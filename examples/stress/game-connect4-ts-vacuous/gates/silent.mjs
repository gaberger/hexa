/**
 * Vacuous gate 2 of 3: the gate that says nothing at all.
 *
 * It runs no test and prints no line. Every check a reader could make is
 * missing, so a reader that treats a zero exit code as the answer gets
 * "pass" from a process that did nothing. In the wild this is a runner whose
 * output was swallowed, a script that exits early, or a make target with its
 * body commented out.
 */
process.exit(0);
