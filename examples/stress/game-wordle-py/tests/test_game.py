"""The game entity: validity, attempt counting, and the verdict."""

import unittest

from src.domain.game import GameOver, Status, new_game
from src.domain.validity import InvalidGuess, is_well_formed, normalize
from src.ports.ids import game_id


def a_game(answer="crane", attempts=6):
    return new_game(game_id("g-1"), answer, max_attempts=attempts)


class ValidityTest(unittest.TestCase):
    def test_a_five_letter_word_is_well_formed(self):
        self.assertTrue(is_well_formed("CRANE"))
        self.assertTrue(is_well_formed("  crane  "))

    def test_wrong_length_or_non_letters_are_not(self):
        self.assertFalse(is_well_formed("cran"))
        self.assertFalse(is_well_formed("cranes"))
        self.assertFalse(is_well_formed("cr4ne"))
        self.assertFalse(is_well_formed("cran "))

    def test_normalize_trims_and_lowercases(self):
        self.assertEqual(normalize("  CrAnE \n"), "crane")


class GameTest(unittest.TestCase):
    def test_a_new_game_is_in_progress_with_every_attempt_left(self):
        game = a_game()
        self.assertIs(game.status, Status.IN_PROGRESS)
        self.assertEqual(game.attempts_left, 6)
        self.assertEqual(game.guesses, ())

    def test_playing_returns_a_new_state_and_leaves_the_old_one_alone(self):
        first = a_game()
        second = first.play("slate")
        self.assertEqual(len(first.guesses), 0)
        self.assertEqual(len(second.guesses), 1)
        self.assertEqual(second.attempts_left, 5)

    def test_a_correct_guess_wins(self):
        game = a_game().play("crane")
        self.assertIs(game.status, Status.WON)
        self.assertTrue(game.is_over())

    def test_running_out_of_attempts_loses(self):
        game = a_game(attempts=2).play("slate").play("brine")
        self.assertIs(game.status, Status.LOST)
        self.assertTrue(game.is_over())

    def test_a_win_on_the_last_attempt_is_a_win_not_a_loss(self):
        game = a_game(attempts=2).play("slate").play("crane")
        self.assertIs(game.status, Status.WON)

    def test_a_guess_after_the_end_is_refused(self):
        game = a_game(attempts=1).play("slate")
        with self.assertRaises(GameOver):
            game.play("brine")

    def test_a_malformed_guess_is_refused(self):
        with self.assertRaises(InvalidGuess):
            a_game().play("cran")

    def test_a_malformed_answer_is_refused_at_creation(self):
        with self.assertRaises(InvalidGuess):
            new_game(game_id("g-2"), "toolong")

    def test_a_game_needs_at_least_one_attempt(self):
        with self.assertRaises(ValueError):
            new_game(game_id("g-3"), "crane", max_attempts=0)

    def test_an_empty_game_id_is_refused(self):
        with self.assertRaises(ValueError):
            game_id("   ")

    def test_marks_for_returns_the_score_of_that_guess(self):
        game = a_game().play("slate").play("crane")
        self.assertEqual(game.marks_for(1), game.guesses[1].score)
        self.assertTrue(game.guesses[1].is_win())
        self.assertFalse(game.guesses[0].is_win())


if __name__ == "__main__":
    unittest.main()
