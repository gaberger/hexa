"""The port a word list is reached through."""

from __future__ import annotations

from typing import Protocol, runtime_checkable

# Re-exported for adapters: an adapter imports the port, never the domain.
from src.domain.validity import WORD_LENGTH, UnknownWord

__all__ = ["WORD_LENGTH", "UnknownWord", "WordSource"]


@runtime_checkable
class WordSource(Protocol):
    """Supplies answers and decides whether a guess is a real word."""

    def pick_answer(self) -> str:
        """Return one answer word, already normalized and well formed."""
        ...

    def contains(self, word: str) -> bool:
        """Is ``word`` in the vocabulary a player may guess?"""
        ...

    def size(self) -> int:
        """How many words the vocabulary holds."""
        ...
