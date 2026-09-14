import type { BoardStore, GameState } from '../ports/board-store.ts';

/** Read a game back. Undefined means there is no such game, not an empty one. */
export function getGame(store: BoardStore, gameId: string): GameState | undefined {
  return store.load(gameId);
}
