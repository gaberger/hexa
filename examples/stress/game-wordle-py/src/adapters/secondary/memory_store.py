"""An in-memory GameStore. Lives as long as the process does."""

from __future__ import annotations

from typing import Dict, Optional

from src.ports.game_store import Game, GameId, GameStore


class InMemoryGameStore(GameStore):
    """A dict behind the GameStore port.

    Subclassing the Protocol is not required - structural typing would do -
    but it makes the intent obvious and catches a renamed method at import.
    """

    def __init__(self) -> None:
        self._games: Dict[str, Game] = dict()

    def save(self, game: Game) -> None:
        self._games[str(game.game_id)] = game

    def load(self, wanted: GameId) -> Optional[Game]:
        return self._games.get(str(wanted))

    def delete(self, wanted: GameId) -> bool:
        return self._games.pop(str(wanted), None) is not None

    def count(self) -> int:
        """How many games are held. Useful to a test, not to the core."""
        return len(self._games)
