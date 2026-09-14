import { createConsoleGame } from './adapters/primary/console-game.ts';
import type { ConsoleGame } from './adapters/primary/console-game.ts';
import { createHeuristicMoveChooser } from './adapters/secondary/heuristic-move-chooser.ts';
import { createInMemoryBoardStore } from './adapters/secondary/in-memory-board-store.ts';
import type { MoveResult } from './domain/game.ts';
import type { GameState } from './domain/types.ts';
import type { BoardStore } from './ports/board-store.ts';
import type { GameApp } from './ports/game-app.ts';
import type { MoveChooser } from './ports/move-chooser.ts';
import { getGame } from './usecases/get-game.ts';
import { playAutoMove } from './usecases/play-auto-move.ts';
import { playMove } from './usecases/play-move.ts';
import { startGame } from './usecases/start-game.ts';

export interface Deps {
  readonly store: BoardStore;
  readonly chooser: MoveChooser;
}

/**
 * The only file that knows an adapter exists.
 *
 * `overrides` lets a test wire a stub chooser without importing an adapter,
 * so the real wiring is itself under test instead of being retyped in a
 * fixture that agrees with it by construction.
 */
export function createConnect4(overrides: Partial<Deps> = {}): GameApp {
  const store = overrides.store ?? createInMemoryBoardStore();
  const chooser = overrides.chooser ?? createHeuristicMoveChooser();

  return {
    startGame: (gameId: string): GameState => startGame(store, gameId),
    playMove: (gameId: string, col: number): MoveResult => playMove(store, gameId, col),
    autoMove: (gameId: string): MoveResult => playAutoMove(store, chooser, gameId),
    getGame: (gameId: string): GameState | undefined => getGame(store, gameId),
  };
}

/** The same application behind its console front end. */
export function createConsole(overrides: Partial<Deps> = {}): ConsoleGame {
  return createConsoleGame(createConnect4(overrides));
}
