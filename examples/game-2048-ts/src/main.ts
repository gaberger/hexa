/**
 * 2048, playable.
 *
 * The project had the whole game — grid, slide, merge, spawn, win and loss —
 * and no way to play it. `bun test` reported 88 passing tests and there was
 * nothing to start, which is the difference between "it compiles" and
 * "it works".
 *
 * This is a primary adapter: it reads keys and writes text. It imports the
 * composition root and nothing else, so the boundary rules still hold.
 */
import { createGame2048 } from './composition-root.ts';
import type { Direction, GameState } from './domain/types.ts';

const SIZE = 4;

function render(state: GameState): string {
  const width = Math.max(
    4,
    ...state.grid.map((c) => String(c === 0 ? '.' : c).length),
  );
  const lines: string[] = [`score ${state.score}   moves ${state.moveCount}`];
  for (let row = 0; row < SIZE; row++) {
    const cells = state.grid
      .slice(row * SIZE, row * SIZE + SIZE)
      .map((c) => String(c === 0 ? '.' : c).padStart(width));
    lines.push(`  ${cells.join(' ')}`);
  }
  return lines.join('\n');
}

const KEYS: Record<string, Direction> = {
  w: 'up',
  a: 'left',
  s: 'down',
  d: 'right',
};

/**
 * Play without a terminal, so a gate can prove the game actually ran.
 *
 * The cycle is fixed rather than random: a demo that plays differently each
 * run cannot be part of a gate.
 */
function demo(): void {
  const game = createGame2048();
  let state = game.startGame('demo');
  const order: Direction[] = ['left', 'up', 'right', 'down'];
  let stuck = 0;
  for (let i = 0; state.status === 'playing' && i < 2000 && stuck < 8; i++) {
    const before = state.grid.join(',');
    state = game.playMove('demo', order[i % order.length]!).state;
    stuck = state.grid.join(',') === before ? stuck + 1 : 0;
  }
  console.log(render(state));
  console.log(state.won ? 'YOU WIN' : 'GAME OVER');
}

function interactive(): void {
  const game = createGame2048();
  let state = game.startGame('you');
  console.log('2048 — w/a/s/d to move, Ctrl-D to quit.');
  console.log(render(state));

  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk: string) => {
    for (const ch of chunk.trim().toLowerCase()) {
      const dir = KEYS[ch];
      if (!dir) continue;
      state = game.playMove('you', dir).state;
    }
    console.log(render(state));
    if (state.status === 'over') {
      console.log(state.won ? 'YOU WIN' : 'GAME OVER');
      process.exit(0);
    }
  });
  process.stdin.on('end', () => {
    console.log(state.won ? 'YOU WIN' : 'GAME OVER');
    process.exit(0);
  });
}

if (process.argv.includes('--demo')) {
  demo();
} else {
  interactive();
}
