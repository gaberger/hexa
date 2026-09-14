"""Identity types shared across the ports.

This module deliberately imports nothing: it is the leaf of the port package
so that the stress violation in ``src/domain/game.py`` (domain importing a
port) does not create an import cycle. The violation is still a violation.
"""

from __future__ import annotations

from typing import NewType

GameId = NewType("GameId", str)


def game_id(raw: str) -> GameId:
    """Build a GameId from a raw string, rejecting the empty one."""
    value = raw.strip()
    if not value:
        raise ValueError("a game id may not be empty")
    return GameId(value)
