"""What counts as a legal guess, and how long a game runs."""

from __future__ import annotations

WORD_LENGTH = 5
MAX_ATTEMPTS = 6


class InvalidGuess(ValueError):
    """Raised when a guess cannot be scored at all."""


class UnknownWord(ValueError):
    """Raised when a well-formed guess is not in the vocabulary."""


def normalize(word: str) -> str:
    """Trim and lowercase; the one place case is decided."""
    return word.strip().lower()


def is_well_formed(word: str) -> bool:
    """Right length, ASCII letters only. Says nothing about the vocabulary."""
    w = normalize(word)
    return len(w) == WORD_LENGTH and w.isalpha() and w.isascii()


def check_well_formed(word: str) -> str:
    """Return the normalized word, or raise :class:`InvalidGuess`."""
    w = normalize(word)
    if not is_well_formed(w):
        raise InvalidGuess(
            repr(word) + " is not a " + str(WORD_LENGTH) + "-letter word"
        )
    return w
