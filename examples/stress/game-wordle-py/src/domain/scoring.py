"""Wordle guess scoring.

The only subtle part of Wordle is duplicate letters. A guess letter earns
YELLOW only if an unmatched copy of that letter still remains in the answer
after every GREEN has claimed its own copy. Greens are resolved in a first
pass, and the answer letters left over form the pool the yellows draw from,
left to right.
"""

from __future__ import annotations

from collections import Counter
from enum import Enum
from typing import Tuple


class Mark(Enum):
    """The colour a single guess position earns."""

    GREEN = "green"
    YELLOW = "yellow"
    GREY = "grey"

    def glyph(self) -> str:
        """A one-character rendering, for terminals without colour."""
        if self is Mark.GREEN:
            return "G"
        if self is Mark.YELLOW:
            return "Y"
        return "."


Score = Tuple[Mark, ...]


def score_guess(answer: str, guess: str) -> Score:
    """Score ``guess`` against ``answer``, position by position.

    Both words are compared case-insensitively and must be the same length.
    """
    answer = answer.lower()
    guess = guess.lower()
    if len(answer) != len(guess):
        raise ValueError(
            "guess " + repr(guess) + " has length " + str(len(guess))
            + ", answer has length " + str(len(answer))
        )

    marks: list[Mark] = [Mark.GREY] * len(guess)

    # Pass 1: exact positions. A letter claimed by a green is unavailable to
    # any later yellow, so it never enters the remaining pool.
    unmatched: Counter[str] = Counter()
    for i, pair in enumerate(zip(answer, guess)):
        a, g = pair
        if a == g:
            marks[i] = Mark.GREEN
        else:
            unmatched[a] += 1

    # Pass 2: misplaced letters, left to right, each consuming one copy.
    for i, g in enumerate(guess):
        if marks[i] is Mark.GREEN:
            continue
        if unmatched[g] > 0:
            marks[i] = Mark.YELLOW
            unmatched[g] -= 1

    return tuple(marks)


def is_winning_score(score: Score) -> bool:
    """A guess wins only when every position is green."""
    return len(score) > 0 and all(m is Mark.GREEN for m in score)


def render_score(score: Score) -> str:
    """One-line rendering of a score, e.g. ``G.Y..``."""
    return "".join(m.glyph() for m in score)
