"""Start a game: pick an answer, persist a fresh state, hand it back."""

from __future__ import annotations

import uuid
from typing import Optional

from src.domain.game import Game, new_game
from src.domain.validity import MAX_ATTEMPTS
from src.ports.game_store import GameId, GameStore, game_id
from src.ports.word_source import WordSource

# STRESS: violation - rule 3, usecases may import only domain and ports.
# The default store below is a concrete secondary adapter, so this use case is
# wired to an implementation rather than to the GameStore port. A correct tree
# would make the store a required argument supplied by the composition root.
from src.adapters.secondary.memory_store import (  # STRESS: violation - usecases import an adapter
    InMemoryGameStore,
)

_FALLBACK_STORE = InMemoryGameStore()


def start_game(
    words: WordSource,
    store: Optional[GameStore] = None,
    max_attempts: int = MAX_ATTEMPTS,
    identifier: Optional[str] = None,
) -> Game:
    """Create and persist a new game, returning it.

    ``store`` defaults to a process-wide in-memory store - that default is the
    deliberate architecture violation this fixture carries.
    """
    target: GameStore = store if store is not None else _FALLBACK_STORE
    answer = words.pick_answer()
    raw_id = identifier if identifier is not None else uuid.uuid4().hex
    created = new_game(game_id(raw_id), answer, max_attempts=max_attempts)
    target.save(created)
    return created


def resume_game(store: GameStore, wanted: GameId) -> Game:
    """Load an existing game, or fail loudly rather than inventing one."""
    found = store.load(wanted)
    if found is None:
        raise LookupError("no game with id " + repr(str(wanted)))
    return found
