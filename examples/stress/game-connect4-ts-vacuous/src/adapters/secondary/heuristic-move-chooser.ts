import { dropPiece, findWinner, legalColumns, opponent } from '../../ports/move-chooser.ts';
import type { GameState, MoveChooser, Player } from '../../ports/move-chooser.ts';

/**
 * Columns in the order this chooser likes them: the middle first.
 *
 * A piece in the middle column sits on more lines than a piece at the edge,
 * so with nothing else to go on the middle is the better guess. Written out
 * by hand: a generated order can share a bug with whatever generated it.
 */
export const PREFERENCE: readonly number[] = Object.freeze([3, 2, 4, 1, 5, 0, 6]);

/** No column is playable. The use case turns this into an ordinary refusal. */
export const NO_COLUMN = -1;

function wins(state: GameState, col: number, player: Player): boolean {
  const drop = dropPiece(state.board, col, player);
  if (drop === null) return false;
  const winner = findWinner(drop.board);
  return winner !== null && winner.player === player;
}

/**
 * Three rules, in order: win now, stop the other side winning now, else take
 * the most central open column.
 *
 * It looks one move ahead and no further, so it is beatable — deliberately.
 * What it is not is random: the same state always gives the same column, so a
 * test can state the answer rather than sample it.
 */
export function createHeuristicMoveChooser(): MoveChooser {
  return {
    choose(state: GameState): number {
      const open = legalColumns(state.board);
      if (open.length === 0) return NO_COLUMN;

      const me = state.toMove;
      const them = opponent(me);

      for (const col of PREFERENCE) {
        if (open.includes(col) && wins(state, col, me)) return col;
      }
      for (const col of PREFERENCE) {
        if (open.includes(col) && wins(state, col, them)) return col;
      }
      for (const col of PREFERENCE) {
        if (open.includes(col)) return col;
      }
      return NO_COLUMN;
    },
  };
}
