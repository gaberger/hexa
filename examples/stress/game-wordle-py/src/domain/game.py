"""The game entity: an answer, the guesses made against it, and the verdict.

Every value here is immutable. Playing a guess returns a new state rather than
mutating the old one, so a store can keep history without defensive copies.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from enum import Enum
from typing import Tuple

from src.domain.scoring import Mark, Score, is_winning_score, score_guess
from src.domain.validity import MAX_ATTEMPTS, check_well_formed

# STRESS: violation - rule 1, domain must import only from domain.
# ``GameId`` lives in the port package, and the domain reaches outward for it.
# A correct tree would define GameId here and let the port import it.
from src.ports.ids import GameId  # STRESS: violation - domain imports ports


class Status(Enum):
    """Where a game stands."""

    IN_PROGRESS = "in_progress"
    WON = "won"
    LOST = "lost"


@dataclass(frozen=True)
class PlayedGuess:
    """One guess and the score it earned."""

    word: str
    score: Score

    def is_win(self) -> bool:
        return is_winning_score(self.score)


@dataclass(frozen=True)
class Game:
    """A single game of Wordle."""

    game_id: GameId
    answer: str
    guesses: Tuple[PlayedGuess, ...] = ()
    max_attempts: int = MAX_ATTEMPTS

    @property
    def status(self) -> Status:
        if self.guesses and self.guesses[-1].is_win():
            return Status.WON
        if len(self.guesses) >= self.max_attempts:
            return Status.LOST
        return Status.IN_PROGRESS

    @property
    def attempts_left(self) -> int:
        return max(0, self.max_attempts - len(self.guesses))

    def is_over(self) -> bool:
        return self.status is not Status.IN_PROGRESS

    def play(self, word: str) -> "Game":
        """Return a new game with ``word`` scored and appended."""
        if self.is_over():
            raise GameOver("this game is already " + self.status.value)
        guess = check_well_formed(word)
        played = PlayedGuess(word=guess, score=score_guess(self.answer, guess))
        return replace(self, guesses=self.guesses + (played,))

    def marks_for(self, index: int) -> Score:
        """The score of the guess at ``index``."""
        return self.guesses[index].score


class GameOver(RuntimeError):
    """Raised when a guess arrives after the game has ended."""


def new_game(game_id: GameId, answer: str, max_attempts: int = MAX_ATTEMPTS) -> Game:
    """Start a game, validating the answer the same way a guess is validated."""
    if max_attempts < 1:
        raise ValueError("a game needs at least one attempt")
    return Game(
        game_id=game_id,
        answer=check_well_formed(answer),
        guesses=(),
        max_attempts=max_attempts,
    )


__all__ = [
    "Game",
    "GameOver",
    "Mark",
    "PlayedGuess",
    "Status",
    "new_game",
]
