import { runSuite } from './run-suite.mjs';

// The real gate. Every compiled file whose name ends in .test.js is run.
runSuite('dist', '.test.js');
