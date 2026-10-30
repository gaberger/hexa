import { atom, read, update } from 'claude-code'
import type { Register } from 'claude-code'

import type { Pending } from '../types'

const pending = atom({ plugin: 'question-alert', key: 'pending' } as const, null as Pending)

// A reply asks something of the person when its closing lines hold a question
// mark or an explicit ask. Code blocks are dropped first so `?` in code is ignored.
const ASK = /\b(want me to|should i|shall i|do you want|would you like|which (option|one)|your call|accept (it|this)|tell me (which|whether|if))\b/i

export function questionIn(answer: string): Pending {
  const prose = answer.replace(/```[\s\S]*?```/g, '').trim()
  const tail = prose.split('\n').filter(l => l.trim() !== '').slice(-4).join(' ')
  if (!tail.includes('?') && !ASK.test(tail)) return null
  const sentences = tail.match(/[^.?!]*[?]/g)
  const q = (sentences?.[sentences.length - 1] ?? tail).replace(/[*_`#>]/g, '').trim()
  return q.length > 160 ? `${q.slice(0, 157)}...` : q
}

export const register: Register = on => {
  on('turn.complete', async ($, e, next) => {
    if (e.agentId === undefined && e.reason === 'answer') {
      const q = questionIn(e.answer)
      await update($, pending, () => q)
      if (q !== null) $.ui.toast(`Claude is asking: ${q}`)
    }
    return next(e)
  })

  on('prompt.submit', async ($, e, next) => {
    // Never hold up the person's prompt over a banner.
    await update($, pending, () => null).catch(() => undefined)
    return next(e)
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const q = await read($, pending)
    if (q === null || e.props.hasSurvey || e.props.isWorking) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box backgroundColor="yellow" paddingX={1}>
        <Text color="black" bold>
          ? Claude is waiting on you: {q}
        </Text>
      </Box>
    )
  })

  on('ui.render', { component: 'AskUserQuestion' }, async ($, e, next) => {
    const own = await next(e)
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box flexDirection="column">
        <Box backgroundColor="yellow" paddingX={1}>
          <Text color="black" bold>
            ? DECISION NEEDED: Claude is asking you
          </Text>
        </Box>
        {own}
      </Box>
    )
  })
}
