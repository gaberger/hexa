import { dropPiece, legalColumns } from '../domain/board.ts';
import { opponent } from '../domain/game.ts';
import { findWinner } from '../domain/win.ts';
import type { GameState, Player } from '../domain/types.ts';

export type { GameState, Player };

/**
 * An adapter imports the port, never the core (hexagonal rule 4). So the
 * pieces of the domain a chooser has to have — the legal columns, a drop it
 * can weigh without playing it, the rule for who wins, the rule for whose
 * turn is next — are re-exported here rather than reached for directly.
 * Otherwise every chooser grows a second edge into the domain.
 */
export { dropPiece, findWinner, legalColumns, opponent };

/**
 * Picks the column to play next.
 *
 * The contract is narrow on purpose: given a state that is still playing, name
 * a legal column. A chooser that returns an illegal column has broken the
 * contract, and the use case refuses the move rather than papering over it.
 */
export interface MoveChooser {
  choose(state: GameState): number;
}
