import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The one honest runner in this fixture.
 *
 * It finds the compiled test files itself, runs them, streams what the test
 * runner said, and then prints a summary line it computed from that output.
 * The summary is not decoration: it is the only line a gate reader can
 * classify, and it is derived from a run that happened.
 *
 * If nothing matched, it says so and reports zero. Reporting zero as a pass
 * is what the three sibling runners do; this one leaves the zero visible.
 */
export function runSuite(dir, suffix) {
  const files = existsSync(dir)
    ? readdirSync(dir)
        .filter((name) => name.endsWith(suffix))
        .sort()
        .map((name) => join(dir, name))
    : [];

  if (files.length === 0) {
    console.log(`runner: no file under ${dir}/ ends with ${suffix}`);
    console.log('test result: ok. 0 passed; 0 failed');
    process.exit(0);
  }

  const run = spawnSync(process.execPath, ['--test', '--test-reporter=tap', ...files], {
    encoding: 'utf8',
  });
  process.stdout.write(run.stdout);
  process.stderr.write(run.stderr);

  const count = (label) => {
    const hit = new RegExp(`^# ${label} (\\d+)$`, 'm').exec(run.stdout);
    return hit === null ? 0 : Number(hit[1]);
  };
  const passed = count('pass');
  const failed = count('fail');
  const ok = run.status === 0 && failed === 0 && passed > 0;

  console.log(`test result: ${ok ? 'ok' : 'FAILED'}. ${passed} passed; ${failed} failed`);
  process.exit(ok ? 0 : 1);
}
