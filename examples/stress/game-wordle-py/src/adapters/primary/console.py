"""A text console front end.

It imports from the ports only. The use cases reach it as two callables the
composition root supplies, so this adapter knows how to talk to a person and
nothing at all about storage, word lists, or scoring.
"""

from __future__ import annotations

from typing import Callable, Optional, Protocol

from src.ports.game_store import Game, Score, Status, render_score

Reader = Callable[[str], str]
Writer = Callable[[str], None]


class StartGame(Protocol):
    """Whatever the composition root passes as "start a game"."""

    def __call__(self) -> Game:
        ...


class SubmitGuess(Protocol):
    """Whatever the composition root passes as "play this word"."""

    def __call__(self, game: Game, word: str) -> "GuessView":
        ...


class GuessView(Protocol):
    """The shape the console needs back from a guess."""

    word: str
    score: Score
    status: Status
    game: Game


class ConsoleGame:
    """Runs a game against a reader and a writer."""

    def __init__(
        self,
        start: StartGame,
        guess: SubmitGuess,
        read: Optional[Reader] = None,
        write: Optional[Writer] = None,
    ) -> None:
        self._start = start
        self._guess = guess
        self._read = read if read is not None else input
        self._write = write if write is not None else print

    def run(self) -> Game:
        """Play one game to its end and return the final state."""
        game = self._start()
        self._write("Wordle. Five letters, " + str(game.max_attempts) + " tries.")
        while not game.is_over():
            entry = self._read("guess> ")
            if entry.strip().lower() in ("quit", "exit"):
                self._write("Answer was " + game.answer + ".")
                return game
            try:
                view = self._guess(game, entry)
            except Exception as failure:
                self._write("! " + str(failure))
                continue
            game = view.game
            self._write(self.format_row(view.word, view.score, game.attempts_left))
        self._write(self.format_verdict(game))
        return game

    @staticmethod
    def format_row(word: str, score: Score, attempts_left: int) -> str:
        """One played line: the word, its marks, and what is left."""
        return (
            word.upper()
            + "  "
            + render_score(score)
            + "  ("
            + str(attempts_left)
            + " left)"
        )

    @staticmethod
    def format_verdict(game: Game) -> str:
        """The closing line of a finished game."""
        if game.status is Status.WON:
            return "Won in " + str(len(game.guesses)) + "."
        return "Lost. The answer was " + game.answer.upper() + "."
