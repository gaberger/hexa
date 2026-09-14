import type { Board, Cell, Player } from './types.ts';

export const COLS = 7;
export const ROWS = 6;
/** Pieces in a line that win the game. */
export const CONNECT = 4;

export const EMPTY_BOARD: Board = Object.freeze(
  new Array<Cell>(ROWS * COLS).fill(null),
);

/** `row * COLS + col`, with the bounds checked. See `Board` for the axes. */
export function indexAt(row: number, col: number): number {
  if (!Number.isInteger(row) || row < 0 || row >= ROWS) {
    throw new RangeError(`row ${row} is outside 0..${ROWS - 1}`);
  }
  if (!Number.isInteger(col) || col < 0 || col >= COLS) {
    throw new RangeError(`column ${col} is outside 0..${COLS - 1}`);
  }
  return row * COLS + col;
}

export function cellAt(board: Board, row: number, col: number): Cell {
  return board[indexAt(row, col)];
}

/** True when `col` names a column of this board. */
export function isColumn(col: number): boolean {
  return Number.isInteger(col) && col >= 0 && col < COLS;
}

/**
 * The row a piece dropped into `col` would come to rest on, or `null` when
 * the column is full. Gravity pulls a piece towards the higher row index, so
 * the search runs from the bottom row upwards.
 */
export function landingRow(board: Board, col: number): number | null {
  for (let row = ROWS - 1; row >= 0; row -= 1) {
    if (board[indexAt(row, col)] === null) return row;
  }
  return null;
}

export interface DropResult {
  readonly board: Board;
  /** The row the piece came to rest on. */
  readonly row: number;
  /** The same square as a flat index, so a caller need not redo the maths. */
  readonly index: number;
}

/** Drop one piece. `null` means the column was full. The board is never mutated. */
export function dropPiece(board: Board, col: number, player: Player): DropResult | null {
  const row = landingRow(board, col);
  if (row === null) return null;
  const index = indexAt(row, col);
  const next = board.slice();
  next[index] = player;
  return { board: Object.freeze(next), row, index };
}

/** The columns a piece can still be dropped into, left to right. */
export function legalColumns(board: Board): number[] {
  const open: number[] = [];
  for (let col = 0; col < COLS; col += 1) {
    if (landingRow(board, col) !== null) open.push(col);
  }
  return open;
}

export function isFull(board: Board): boolean {
  return board.every((cell) => cell !== null);
}
