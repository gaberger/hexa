import { newGame } from '../domain/game.ts';
import type { BoardStore, GameState } from '../ports/board-store.ts';

/** The caller supplies the id. The program never invents one. */
export function startGame(store: BoardStore, gameId: string): GameState {
  if (store.load(gameId) !== undefined) {
    throw new Error(`a game with the id "${gameId}" already exists`);
  }
  const state = newGame();
  store.save(gameId, state);
  return state;
}
