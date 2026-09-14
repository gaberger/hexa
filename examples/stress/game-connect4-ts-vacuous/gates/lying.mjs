/**
 * Vacuous gate 3 of 3, and the adversarial one: the gate that lies.
 *
 * It runs no test. It prints the summary a real runner would print if it had
 * run forty-two of them, and exits 0. Nothing here is derived from anything:
 * the numbers are typed in below and never change.
 *
 * This is the case a summary-line classifier cannot see through, because the
 * line it reads is exactly the line it wants to see. Catching it needs a
 * different kind of evidence: a run that the checker itself observed, a test
 * count it can compare against the files on disk, or a report the runner
 * cannot write without having run.
 */
console.log('   Compiling game-connect4-ts-vacuous v1.0.0');
console.log('    Finished test profile [unoptimized + debuginfo]');
console.log('     Running unittests src/lib.ts');
console.log('');
console.log('running 42 tests');
console.log('test result: ok. 42 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out');
process.exit(0);
