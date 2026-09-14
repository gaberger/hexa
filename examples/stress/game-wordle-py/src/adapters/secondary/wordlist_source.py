"""A WordSource backed by a fixed word list, with no third-party dependency."""

from __future__ import annotations

import random
from typing import Iterable, List, Optional, Sequence

from src.ports.word_source import WORD_LENGTH, UnknownWord, WordSource

DEFAULT_WORDS: Sequence[str] = (
    "crane", "slate", "audio", "brine", "chase", "dwell", "eagle", "flint",
    "grape", "hoist", "inlet", "jolly", "knead", "llama", "mirth", "noble",
    "ovals", "pride", "quilt", "raise", "sever", "trove", "usher", "vivid",
    "wharf", "xenon", "yeast", "zebra", "allay", "geese", "ferry", "esses",
    "sassy", "banal", "abbey", "melee", "wooed", "erase", "eerie", "array",
)


class WordlistWordSource(WordSource):
    """Picks answers from a list, and answers membership questions about it."""

    def __init__(
        self,
        words: Optional[Iterable[str]] = None,
        rng: Optional[random.Random] = None,
    ) -> None:
        source = DEFAULT_WORDS if words is None else words
        cleaned: List[str] = []
        for raw in source:
            word = raw.strip().lower()
            if len(word) != WORD_LENGTH or not word.isalpha():
                raise ValueError("word list holds a bad entry: " + repr(raw))
            cleaned.append(word)
        if not cleaned:
            raise ValueError("a word source needs at least one word")
        # Sorted and de-duplicated, so membership and picking are stable.
        self._words: List[str] = sorted(set(cleaned))
        self._rng = rng if rng is not None else random.Random()

    def pick_answer(self) -> str:
        return self._rng.choice(self._words)

    def contains(self, word: str) -> bool:
        return word.strip().lower() in self._words

    def size(self) -> int:
        return len(self._words)

    def require(self, word: str) -> str:
        """Return the normalized word, or raise UnknownWord."""
        candidate = word.strip().lower()
        if candidate not in self._words:
            raise UnknownWord(repr(word) + " is not in the word list")
        return candidate
