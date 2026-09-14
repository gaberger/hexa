"""The adapters and the wiring, end to end, with no terminal involved."""

import random
import unittest

from src.adapters.primary.console import ConsoleGame
from src.adapters.secondary.memory_store import InMemoryGameStore
from src.adapters.secondary.wordlist_source import (
    DEFAULT_WORDS,
    WordlistWordSource,
)
from src.composition_root import build
from src.domain.game import Status, new_game
from src.domain.scoring import Mark
from src.ports.ids import game_id

VOCAB = ("crane", "slate", "brine", "abbey", "allay")


class PinnedRandom(random.Random):
    def __init__(self, answer):
        super().__init__()
        self._answer = answer

    def choice(self, seq):
        return self._answer


class InMemoryGameStoreTest(unittest.TestCase):
    def test_save_then_load_returns_the_same_game(self):
        store = InMemoryGameStore()
        game = new_game(game_id("g-1"), "crane")
        store.save(game)
        self.assertEqual(store.load(game_id("g-1")), game)

    def test_loading_an_absent_game_returns_none(self):
        self.assertIsNone(InMemoryGameStore().load(game_id("nope")))

    def test_saving_the_same_id_twice_replaces_the_state(self):
        store = InMemoryGameStore()
        game = new_game(game_id("g-1"), "crane")
        store.save(game)
        store.save(game.play("slate"))
        self.assertEqual(len(store.load(game_id("g-1")).guesses), 1)
        self.assertEqual(store.count(), 1)

    def test_delete_reports_whether_it_removed_anything(self):
        store = InMemoryGameStore()
        store.save(new_game(game_id("g-1"), "crane"))
        self.assertTrue(store.delete(game_id("g-1")))
        self.assertFalse(store.delete(game_id("g-1")))


class WordlistWordSourceTest(unittest.TestCase):
    def test_membership_is_case_and_space_insensitive(self):
        words = WordlistWordSource(words=VOCAB)
        self.assertTrue(words.contains("  CRANE "))
        self.assertFalse(words.contains("zzzzz"))

    def test_duplicates_collapse_and_size_reports_the_truth(self):
        words = WordlistWordSource(words=("crane", "crane", "slate"))
        self.assertEqual(words.size(), 2)

    def test_a_bad_entry_is_refused_at_construction(self):
        with self.assertRaises(ValueError):
            WordlistWordSource(words=("crane", "toolong"))
        with self.assertRaises(ValueError):
            WordlistWordSource(words=())

    def test_the_shipped_list_is_all_five_letter_words(self):
        words = WordlistWordSource()
        self.assertEqual(words.size(), len(set(DEFAULT_WORDS)))
        self.assertTrue(all(len(w) == 5 for w in DEFAULT_WORDS))

    def test_a_seeded_rng_makes_the_answer_reproducible(self):
        first = WordlistWordSource(rng=random.Random(7)).pick_answer()
        second = WordlistWordSource(rng=random.Random(7)).pick_answer()
        self.assertEqual(first, second)


class ConsoleGameTest(unittest.TestCase):
    def play(self, answer, entries):
        wiring = build(vocabulary=VOCAB)
        wiring.words = WordlistWordSource(words=VOCAB, rng=PinnedRandom(answer))
        typed = iter(entries)
        self.printed = []
        console = ConsoleGame(
            start=wiring.start,
            guess=wiring.guess,
            read=lambda prompt: next(typed),
            write=self.printed.append,
        )
        return console.run()

    def test_a_won_game_reports_a_win(self):
        game = self.play("crane", ["slate", "crane"])
        self.assertIs(game.status, Status.WON)
        self.assertIn("Won in 2.", self.printed)

    def test_a_lost_game_reveals_the_answer(self):
        entries = ["slate", "brine", "abbey", "allay", "slate", "brine"]
        game = self.play("crane", entries)
        self.assertIs(game.status, Status.LOST)
        self.assertIn("Lost. The answer was CRANE.", self.printed)

    def test_a_rejected_word_is_reported_and_the_game_goes_on(self):
        game = self.play("crane", ["zzzzz", "crane"])
        self.assertIs(game.status, Status.WON)
        self.assertTrue(any(line.startswith("! ") for line in self.printed))

    def test_quitting_ends_the_game_without_a_guess(self):
        game = self.play("crane", ["quit"])
        self.assertIs(game.status, Status.IN_PROGRESS)
        self.assertEqual(len(game.guesses), 0)

    def test_the_played_row_shows_word_marks_and_attempts_left(self):
        row = ConsoleGame.format_row("slate", (Mark.GREY, Mark.GREEN), 4)
        self.assertEqual(row, "SLATE  .G  (4 left)")


class WiringTest(unittest.TestCase):
    def test_build_with_a_seed_is_reproducible(self):
        first = build(seed=3).start().answer
        second = build(seed=3).start().answer
        self.assertEqual(first, second)

    def test_the_wiring_stores_the_game_it_starts(self):
        wiring = build(seed=1)
        game = wiring.start()
        self.assertIsNotNone(wiring.store.load(game.game_id))

    def test_the_wiring_plays_a_guess_through_the_use_case(self):
        wiring = build(seed=1, vocabulary=VOCAB)
        game = wiring.start()
        result = wiring.guess(game, "slate")
        self.assertEqual(result.word, "slate")
        self.assertEqual(len(wiring.store.load(game.game_id).guesses), 1)


if __name__ == "__main__":
    unittest.main()
