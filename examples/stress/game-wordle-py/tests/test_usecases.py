"""The use cases, against the real in-memory adapters."""

import random
import unittest

from src.adapters.secondary.memory_store import InMemoryGameStore
from src.adapters.secondary.wordlist_source import WordlistWordSource
from src.domain.game import GameOver, Status
from src.domain.validity import InvalidGuess, UnknownWord
from src.ports.ids import game_id
from src.usecases.start_game import resume_game, start_game
from src.usecases.submit_guess import submit_guess

VOCAB = ("crane", "slate", "brine", "abbey", "allay")


class PinnedRandom(random.Random):
    """Always picks one named word, so a test can know the answer."""

    def __init__(self, answer):
        super().__init__()
        self._answer = answer

    def choice(self, seq):
        if self._answer not in seq:
            raise ValueError("pinned answer is not in the word list")
        return self._answer


def fixed_source(answer="crane"):
    # The vocabulary stays wide - a one-word list would make "is this word
    # known" vacuously true - and the answer is pinned by the rng instead.
    return WordlistWordSource(words=VOCAB, rng=PinnedRandom(answer))


class StartGameTest(unittest.TestCase):
    def test_starting_persists_a_game_that_can_be_resumed(self):
        store = InMemoryGameStore()
        game = start_game(fixed_source("crane"), store, identifier="g-1")
        self.assertEqual(store.count(), 1)
        self.assertEqual(resume_game(store, game.game_id).answer, "crane")

    def test_the_answer_comes_from_the_word_source(self):
        store = InMemoryGameStore()
        game = start_game(fixed_source("abbey"), store, identifier="g-2")
        self.assertEqual(game.answer, "abbey")

    def test_two_games_get_different_ids_by_default(self):
        store = InMemoryGameStore()
        words = fixed_source("crane")
        first = start_game(words, store)
        second = start_game(words, store)
        self.assertNotEqual(first.game_id, second.game_id)
        self.assertEqual(store.count(), 2)

    def test_resuming_an_unknown_game_fails_loudly(self):
        with self.assertRaises(LookupError):
            resume_game(InMemoryGameStore(), game_id("nope"))


class SubmitGuessTest(unittest.TestCase):
    def setUp(self):
        self.store = InMemoryGameStore()
        self.words = fixed_source("crane")
        self.game = start_game(
            self.words, self.store, max_attempts=3, identifier="g-1"
        )

    def test_a_guess_is_scored_and_the_new_state_persisted(self):
        result = submit_guess(self.store, self.words, self.game.game_id, "slate")
        self.assertEqual(result.word, "slate")
        self.assertEqual(len(result.score), 5)
        self.assertIs(result.status, Status.IN_PROGRESS)
        reloaded = self.store.load(self.game.game_id)
        self.assertEqual(len(reloaded.guesses), 1)
        self.assertEqual(result.attempts_left, 2)

    def test_the_right_word_wins_and_ends_the_game(self):
        result = submit_guess(self.store, self.words, self.game.game_id, "CRANE")
        self.assertIs(result.status, Status.WON)
        self.assertTrue(result.is_over)

    def test_running_out_of_attempts_loses(self):
        result = None
        for word in ("slate", "brine", "abbey"):
            result = submit_guess(self.store, self.words, self.game.game_id, word)
        self.assertIs(result.status, Status.LOST)
        self.assertEqual(result.attempts_left, 0)

    def test_a_word_outside_the_vocabulary_is_refused_and_costs_no_attempt(self):
        with self.assertRaises(UnknownWord):
            submit_guess(self.store, self.words, self.game.game_id, "zzzzz")
        self.assertEqual(len(self.store.load(self.game.game_id).guesses), 0)

    def test_a_malformed_word_is_refused(self):
        with self.assertRaises(InvalidGuess):
            submit_guess(self.store, self.words, self.game.game_id, "cran")

    def test_guessing_in_a_finished_game_is_refused(self):
        submit_guess(self.store, self.words, self.game.game_id, "crane")
        with self.assertRaises(GameOver):
            submit_guess(self.store, self.words, self.game.game_id, "slate")

    def test_an_unknown_game_is_a_lookup_error(self):
        with self.assertRaises(LookupError):
            submit_guess(self.store, self.words, game_id("missing"), "slate")


if __name__ == "__main__":
    unittest.main()
