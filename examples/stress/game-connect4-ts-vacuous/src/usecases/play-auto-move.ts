import { applyMove } from '../domain/game.ts';
import type { MoveResult } from '../domain/game.ts';
import type { BoardStore } from '../ports/board-store.ts';
import type { MoveChooser } from '../ports/move-chooser.ts';

/**
 * Let the chooser answer for the side to move.
 *
 * The chooser is a port, so it can be wrong. An illegal column comes back as
 * an ordinary refusal from the domain and the stored game does not move. The
 * use case does not second-guess the chooser and it does not pick again.
 */
export function playAutoMove(
  store: BoardStore,
  chooser: MoveChooser,
  gameId: string,
): MoveResult {
  const state = store.load(gameId);
  if (state === undefined) {
    throw new Error(`no game with the id "${gameId}"`);
  }
  const result = applyMove(state, chooser.choose(state));
  store.save(gameId, result.state);
  return result;
}
