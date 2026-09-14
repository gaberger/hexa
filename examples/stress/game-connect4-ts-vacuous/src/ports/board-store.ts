import type { GameState } from '../domain/types.ts';

export type { GameState };

/**
 * Where a game is kept between moves.
 *
 * Every call answers at once. Nothing here is async, so no second caller can
 * cut in half way through a move and save over it.
 */
export interface BoardStore {
  load(gameId: string): GameState | undefined;
  save(gameId: string, state: GameState): void;
  /** Without this, a Map keeps every game ever played for the life of the process. */
  remove(gameId: string): void;
}
