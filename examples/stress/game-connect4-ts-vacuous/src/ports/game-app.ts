import { COLS, ROWS, cellAt } from '../domain/board.ts';
import type { MoveResult } from '../domain/game.ts';
import type { Cell, GameState, Player } from '../domain/types.ts';

export type { Cell, GameState, MoveResult, Player };

/** The board shape and one reader, so a view can draw without touching the core. */
export { COLS, ROWS, cellAt };

/**
 * What a driving adapter is allowed to ask of the application.
 *
 * The console adapter is written against this and nothing else. The
 * composition root is the only place that knows a use case, a store or a
 * chooser exists.
 */
export interface GameApp {
  startGame(gameId: string): GameState;
  playMove(gameId: string, col: number): MoveResult;
  /** Let the built-in chooser answer for the side to move. */
  autoMove(gameId: string): MoveResult;
  getGame(gameId: string): GameState | undefined;
}
