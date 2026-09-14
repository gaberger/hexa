import { applyMove } from '../domain/game.ts';
import type { MoveResult } from '../domain/game.ts';
import type { BoardStore } from '../ports/board-store.ts';

/**
 * Play one move and keep the result.
 *
 * A refused move leaves the stored game exactly as it was: the domain hands
 * back the state it was given, so saving the result either way is safe and
 * there is no branch here to get wrong.
 */
export function playMove(store: BoardStore, gameId: string, col: number): MoveResult {
  const state = store.load(gameId);
  if (state === undefined) {
    throw new Error(`no game with the id "${gameId}"`);
  }
  const result = applyMove(state, col);
  store.save(gameId, result.state);
  return result;
}
