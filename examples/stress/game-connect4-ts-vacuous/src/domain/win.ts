import { CONNECT, COLS, ROWS, cellAt, indexAt } from './board.ts';
import type { Board, Player } from './types.ts';

/**
 * A step of one square, as `[dRow, dCol]`.
 *
 * Signs follow the coordinate system in `types.ts`: row grows downwards, col
 * grows to the right. So `[1, 1]` walks down-and-right — the `\` diagonal —
 * and `[1, -1]` walks down-and-left — the `/` diagonal. Writing the sign
 * convention down is the whole reason this type exists; the two diagonals are
 * where a Connect Four is usually gotten wrong.
 */
export type Step = readonly [number, number];

export const DIRECTIONS: readonly Step[] = Object.freeze([
  Object.freeze([0, 1]) as Step, // horizontal, along a row
  Object.freeze([1, 0]) as Step, // vertical, down a column
  Object.freeze([1, 1]) as Step, // diagonal, down-right: the `\` line
  Object.freeze([1, -1]) as Step, // diagonal, down-left: the `/` line
]);

function inBounds(row: number, col: number): boolean {
  return row >= 0 && row < ROWS && col >= 0 && col < COLS;
}

/**
 * The winning run through one square, or `null`.
 *
 * This is the cheap check the game runs after every drop: only the square
 * that just changed can have started winning. It walks backwards to the end
 * of the run and then forwards, so a run longer than `CONNECT` comes back
 * whole. Indices are in board order.
 */
export function winningLineThrough(board: Board, row: number, col: number): readonly number[] | null {
  const player = cellAt(board, row, col);
  if (player === null) return null;

  for (const [dRow, dCol] of DIRECTIONS) {
    let startRow = row;
    let startCol = col;
    while (
      inBounds(startRow - dRow, startCol - dCol) &&
      cellAt(board, startRow - dRow, startCol - dCol) === player
    ) {
      startRow -= dRow;
      startCol -= dCol;
    }

    const run: number[] = [];
    let r = startRow;
    let c = startCol;
    while (inBounds(r, c) && cellAt(board, r, c) === player) {
      run.push(indexAt(r, c));
      r += dRow;
      c += dCol;
    }

    if (run.length >= CONNECT) return Object.freeze([...run].sort((a, b) => a - b));
  }

  return null;
}

export interface Winner {
  readonly player: Player;
  readonly line: readonly number[];
}

/**
 * The winner on a board, found by scanning the whole board.
 *
 * Deliberately a second, differently shaped implementation of the same rule:
 * it starts a window at every square in every direction instead of walking
 * out from one square. A caller that holds a board it did not build move by
 * move — the move chooser, weighing a drop it has not played — needs this
 * one. Having two lets the tests check each against the other; a single
 * implementation can only be checked against the expectations of whoever
 * wrote it.
 */
export function findWinner(board: Board): Winner | null {
  for (let row = 0; row < ROWS; row += 1) {
    for (let col = 0; col < COLS; col += 1) {
      const player = cellAt(board, row, col);
      if (player === null) continue;

      for (const [dRow, dCol] of DIRECTIONS) {
        const line: number[] = [];
        for (let k = 0; k < CONNECT; k += 1) {
          const r = row + k * dRow;
          const c = col + k * dCol;
          if (!inBounds(r, c) || cellAt(board, r, c) !== player) break;
          line.push(indexAt(r, c));
        }
        if (line.length === CONNECT) {
          return { player, line: Object.freeze([...line].sort((a, b) => a - b)) };
        }
      }
    }
  }
  return null;
}
