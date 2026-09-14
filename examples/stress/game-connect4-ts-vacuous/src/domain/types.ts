/** The two sides. Red always moves first. */
export type Player = 'red' | 'yellow';

/** A square holds a player's piece, or `null` when it is empty. */
export type Cell = Player | null;

/**
 * Forty-two squares, flat.
 *
 * The coordinate system is written down once, here, and nothing else gets to
 * disagree with it: `index = row * COLS + col`. Row 0 is the TOP row and row
 * `ROWS - 1` is the BOTTOM row — the one a piece reaches when its column is
 * empty. Column 0 is the left edge. A falling piece moves towards a HIGHER
 * row index.
 */
export type Board = readonly Cell[];

export type GameStatus = 'playing' | 'won' | 'draw';

export interface GameState {
  readonly board: Board;
  /** Whose turn it is. It stops changing once `status` leaves `playing`. */
  readonly toMove: Player;
  readonly status: GameStatus;
  /** Set only when `status` is `won`. */
  readonly winner: Player | null;
  /** The four board indices that ended it, in board order. */
  readonly winningLine: readonly number[] | null;
  readonly moveCount: number;
}
