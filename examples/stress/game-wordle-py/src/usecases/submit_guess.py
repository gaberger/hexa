"""Submit one guess against a stored game."""

from __future__ import annotations

from dataclasses import dataclass

from src.domain.game import Game, Status
from src.domain.scoring import Score
from src.domain.validity import UnknownWord, check_well_formed
from src.ports.game_store import GameId, GameStore
from src.ports.word_source import WordSource


@dataclass(frozen=True)
class GuessResult:
    """What one guess produced: the score, the new state, and the verdict."""

    game: Game
    word: str
    score: Score
    status: Status

    @property
    def attempts_left(self) -> int:
        return self.game.attempts_left

    @property
    def is_over(self) -> bool:
        return self.status is not Status.IN_PROGRESS


def submit_guess(
    store: GameStore,
    words: WordSource,
    wanted: GameId,
    raw_guess: str,
) -> GuessResult:
    """Score ``raw_guess`` against the stored game and persist the result.

    Raises LookupError for an unknown game, InvalidGuess for a malformed word,
    UnknownWord for a well-formed word that is not in the vocabulary, and
    GameOver when the game has already finished.
    """
    game = store.load(wanted)
    if game is None:
        raise LookupError("no game with id " + repr(str(wanted)))

    guess = check_well_formed(raw_guess)
    if not words.contains(guess):
        raise UnknownWord(repr(raw_guess) + " is not in the word list")

    played = game.play(guess)
    store.save(played)
    return GuessResult(
        game=played,
        word=guess,
        score=played.guesses[-1].score,
        status=played.status,
    )
