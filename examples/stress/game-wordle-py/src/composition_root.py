"""The single wiring point: adapters bound to ports, then the game run.

This is the only file that is allowed to import an adapter. One of the two
deliberate stress violations breaks that - see ``src/usecases/start_game.py``.
"""

from __future__ import annotations

import random
import sys
from typing import Iterable, Optional

from src.adapters.primary.console import ConsoleGame
from src.adapters.secondary.memory_store import InMemoryGameStore
from src.adapters.secondary.wordlist_source import WordlistWordSource
from src.domain.game import Game
from src.ports.game_store import GameStore
from src.ports.word_source import WordSource
from src.usecases.start_game import start_game
from src.usecases.submit_guess import GuessResult, submit_guess


class Wiring:
    """Everything the console needs, built once."""

    def __init__(
        self,
        store: Optional[GameStore] = None,
        words: Optional[WordSource] = None,
    ) -> None:
        self.store: GameStore = store if store is not None else InMemoryGameStore()
        self.words: WordSource = (
            words if words is not None else WordlistWordSource()
        )

    def start(self) -> Game:
        return start_game(self.words, self.store)

    def guess(self, game: Game, word: str) -> GuessResult:
        return submit_guess(self.store, self.words, game.game_id, word)

    def console(self) -> ConsoleGame:
        return ConsoleGame(start=self.start, guess=self.guess)


def build(
    seed: Optional[int] = None,
    vocabulary: Optional[Iterable[str]] = None,
) -> Wiring:
    """Build a wiring, optionally seeded so a run is reproducible."""
    rng = random.Random(seed) if seed is not None else None
    return Wiring(
        store=InMemoryGameStore(),
        words=WordlistWordSource(words=vocabulary, rng=rng),
    )


def main(argv: Optional[list[str]] = None) -> int:
    """Entry point: ``python3 -m src.composition_root [seed]``."""
    args = argv if argv is not None else sys.argv[1:]
    seed = int(args[0]) if args else None
    final = build(seed=seed).console().run()
    return 0 if final.status.value != "lost" else 1


if __name__ == "__main__":
    raise SystemExit(main())
