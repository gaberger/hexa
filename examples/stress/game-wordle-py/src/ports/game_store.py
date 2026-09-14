"""The port a game is persisted through.

Rule 4 has a consequence: an adapter that needs a domain type imports it from
the port, not from the domain. So the domain types an adapter touches are
re-exported here.
"""

from __future__ import annotations

from typing import Optional, Protocol, runtime_checkable

from src.domain.game import Game, GameOver, PlayedGuess, Status
from src.domain.scoring import Mark, Score, render_score
from src.ports.ids import GameId, game_id

__all__ = [
    "Game",
    "GameId",
    "GameOver",
    "GameStore",
    "Mark",
    "PlayedGuess",
    "Score",
    "Status",
    "game_id",
    "render_score",
]


@runtime_checkable
class GameStore(Protocol):
    """Keeps games between commands."""

    def save(self, game: Game) -> None:
        """Persist ``game`` under its own id, replacing any earlier state."""
        ...

    def load(self, wanted: GameId) -> Optional[Game]:
        """Return the game with that id, or None if there is none."""
        ...

    def delete(self, wanted: GameId) -> bool:
        """Forget a game. True if something was actually removed."""
        ...
