import { expect, test } from 'claude-code/testing'

import { questionIn } from '../hooks/register'

test('a closing question is found', async () => {
  expect(questionIn('Done with Phase 1.\n\nWant me to start Phase 2?')).toBe('Want me to start Phase 2?')
  expect(questionIn('All good.\nShould I push this to git.local')).not.toBe(null)
})

test('a plain status reply raises nothing', async () => {
  expect(questionIn('Phase 0 is done and pushed. The gate passes.')).toBe(null)
  expect(questionIn('Built it.\n```rust\nlet x = a?;\n```\nAll tests pass.')).toBe(null)
})
