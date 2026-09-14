import { EMPTY_BOARD, dropPiece, isColumn, isFull } from './board.ts';
import { winningLineThrough } from './win.ts';
import type { GameState, Player } from './types.ts';

/** Red moves first. The rule lives here and nowhere else. */
export const FIRST_PLAYER: Player = 'red';

export function opponent(player: Player): Player {
  return player === 'red' ? 'yellow' : 'red';
}

export function newGame(): GameState {
  return Object.freeze({
    board: EMPTY_BOARD,
    toMove: FIRST_PLAYER,
    status: 'playing',
    winner: null,
    winningLine: null,
    moveCount: 0,
  }) as GameState;
}

export type MoveOutcome = 'played' | 'column-full' | 'no-such-column' | 'game-over';

export interface MoveResult {
  /** The state after the move, or the state unchanged when the move was refused. */
  readonly state: GameState;
  readonly outcome: MoveOutcome;
  /** Where the piece landed, as a flat index. Null when nothing was played. */
  readonly droppedAt: number | null;
}

/**
 * Play one piece into a column.
 *
 * A refused move returns the state it was given, so a caller that saves the
 * result unconditionally cannot corrupt a game with a bad column. The three
 * refusals stay distinct because a caller that cannot tell "that column is
 * full" from "there is no such column" cannot tell a player either.
 */
export function applyMove(state: GameState, col: number): MoveResult {
  if (state.status !== 'playing') {
    return { state, outcome: 'game-over', droppedAt: null };
  }
  if (!isColumn(col)) {
    return { state, outcome: 'no-such-column', droppedAt: null };
  }

  const drop = dropPiece(state.board, col, state.toMove);
  if (drop === null) {
    return { state, outcome: 'column-full', droppedAt: null };
  }

  const line = winningLineThrough(drop.board, drop.row, col);
  const moveCount = state.moveCount + 1;

  if (line !== null) {
    const won = Object.freeze({
      board: drop.board,
      toMove: state.toMove,
      status: 'won',
      winner: state.toMove,
      winningLine: line,
      moveCount,
    }) as GameState;
    return { state: won, outcome: 'played', droppedAt: drop.index };
  }

  if (isFull(drop.board)) {
    const drawn = Object.freeze({
      board: drop.board,
      toMove: state.toMove,
      status: 'draw',
      winner: null,
      winningLine: null,
      moveCount,
    }) as GameState;
    return { state: drawn, outcome: 'played', droppedAt: drop.index };
  }

  const next = Object.freeze({
    board: drop.board,
    toMove: opponent(state.toMove),
    status: 'playing',
    winner: null,
    winningLine: null,
    moveCount,
  }) as GameState;
  return { state: next, outcome: 'played', droppedAt: drop.index };
}
