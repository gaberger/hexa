import { COLS, ROWS, cellAt } from '../../ports/game-app.ts';
import type { Cell, GameApp, GameState, MoveResult } from '../../ports/game-app.ts';

const MARK: Readonly<Record<string, string>> = Object.freeze({ red: 'R', yellow: 'Y' });
const EMPTY_MARK = '.';

function mark(cell: Cell): string {
  if (cell === null) return EMPTY_MARK;
  return MARK[cell];
}

/** The board as text, top row first, with the column numbers a player types. */
export function renderBoard(state: GameState): string[] {
  const lines: string[] = [];
  for (let row = 0; row < ROWS; row += 1) {
    const squares: string[] = [];
    for (let col = 0; col < COLS; col += 1) {
      squares.push(mark(cellAt(state.board, row, col)));
    }
    lines.push(`| ${squares.join(' ')} |`);
  }
  const numbers: string[] = [];
  for (let col = 0; col < COLS; col += 1) numbers.push(String(col + 1));
  lines.push(`  ${numbers.join(' ')}  `);
  return lines;
}

/** One line saying where the game stands. */
export function renderStatus(state: GameState): string {
  if (state.status === 'won') return `${state.winner} wins in ${state.moveCount} moves`;
  if (state.status === 'draw') return `a draw after ${state.moveCount} moves`;
  return `${state.toMove} to move`;
}

function renderOutcome(result: MoveResult): string[] {
  switch (result.outcome) {
    case 'column-full':
      return ['that column is full, pick another'];
    case 'no-such-column':
      return [`pick a column from 1 to ${COLS}`];
    case 'game-over':
      return ['this game is over, start a new one'];
    default:
      return [...renderBoard(result.state), renderStatus(result.state)];
  }
}

export interface ConsoleGame {
  /** Start a game and draw it. */
  start(gameId: string): string[];
  /** Take one typed line and answer with the lines to print. */
  handle(gameId: string, input: string): string[];
}

export const HELP = 'type 1-7 to drop a piece, auto to let the machine move, board to redraw';

/**
 * A console front end.
 *
 * It holds no game state of its own and it parses; that is all a driving
 * adapter is for. It returns the lines instead of printing them, so the
 * thing that is easy to get wrong — what a player sees — is the thing a test
 * can read back.
 */
export function createConsoleGame(app: GameApp): ConsoleGame {
  return {
    start(gameId: string): string[] {
      const state = app.startGame(gameId);
      return [HELP, ...renderBoard(state), renderStatus(state)];
    },

    handle(gameId: string, input: string): string[] {
      const word = input.trim().toLowerCase();

      if (word === 'board') {
        const state = app.getGame(gameId);
        if (state === undefined) return ['no such game'];
        return [...renderBoard(state), renderStatus(state)];
      }

      if (word === 'auto') {
        return renderOutcome(app.autoMove(gameId));
      }

      if (/^[0-9]+$/.test(word)) {
        return renderOutcome(app.playMove(gameId, Number(word) - 1));
      }

      return [HELP];
    },
  };
}
