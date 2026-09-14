"""Scoring, with the duplicate-letter cases written out by hand.

These are unittest.TestCase classes so the suite runs under both
``python3 -m pytest`` and ``python3 -m unittest discover``.
"""

import itertools
import unittest

from src.domain.scoring import (
    Mark,
    is_winning_score,
    render_score,
    score_guess,
)

G = Mark.GREEN
Y = Mark.YELLOW
X = Mark.GREY


class ScoreGuessTest(unittest.TestCase):
    def test_exact_match_is_all_green(self):
        self.assertEqual(score_guess("crane", "crane"), (G, G, G, G, G))

    def test_no_shared_letters_is_all_grey(self):
        self.assertEqual(score_guess("crane", "spilt"), (X, X, X, X, X))

    def test_shared_letters_in_place_are_green(self):
        # c r a n e  answer
        # b r i n e  guess -> r, n and e sit in their own positions.
        self.assertEqual(score_guess("crane", "brine"), (X, G, X, G, G))

    def test_a_letter_present_but_elsewhere_is_yellow(self):
        self.assertEqual(score_guess("abcde", "eabcd"), (Y, Y, Y, Y, Y))

    def test_case_is_ignored(self):
        self.assertEqual(score_guess("CRANE", "CrAnE"), (G, G, G, G, G))

    def test_length_mismatch_is_an_error(self):
        with self.assertRaises(ValueError):
            score_guess("crane", "crates")

    def test_render_is_one_glyph_per_position(self):
        self.assertEqual(render_score(score_guess("crane", "brine")), ".G.GG")

    def test_only_all_green_wins(self):
        self.assertTrue(is_winning_score(score_guess("crane", "crane")))
        self.assertFalse(is_winning_score(score_guess("crane", "brine")))
        self.assertFalse(is_winning_score(()))


class DuplicateLetterTest(unittest.TestCase):
    """The rule: a guess letter earns yellow only if an unmatched copy remains."""

    def test_extra_copies_in_the_guess_get_nothing(self):
        # a b b e y  answer, one 'e'
        # e e r i e  guess, three 'e' and no green anywhere.
        # The leftmost 'e' takes the single copy; the other two stay grey.
        self.assertEqual(score_guess("abbey", "eerie"), (Y, X, X, X, X))

    def test_a_green_elsewhere_still_leaves_one_copy_for_a_yellow(self):
        # m e l e e  answer, three 'e'
        # e a t e n  guess: index 3 is green, index 0 draws from the two left.
        self.assertEqual(score_guess("melee", "eaten"), (Y, X, X, G, X))

    def test_yellows_are_handed_out_left_to_right(self):
        # g e e s e  answer, one 's'
        # e s s e s  guess, three 's': only the leftmost earns yellow.
        self.assertEqual(score_guess("geese", "esses"), (Y, Y, X, Y, X))

    def test_greens_claim_their_copy_before_any_yellow(self):
        # a l l a y  answer, two 'l'
        # l o l l y  guess, three 'l': index 2 is green and index 0 takes the
        # second copy, so index 3 is grey even though the answer has an 'l'.
        self.assertEqual(score_guess("allay", "lolly"), (Y, X, G, X, G))

    def test_two_greens_exhaust_both_copies(self):
        # a r r a y  answer, two 'r'
        # r r r r a  guess: the greens at 1 and 2 take both 'r', so the 'r'
        # at 0 and 3 are grey; the trailing 'a' is yellow.
        self.assertEqual(score_guess("array", "rrrra"), (X, G, G, X, Y))

    def test_all_duplicates_matched_exactly(self):
        self.assertEqual(score_guess("esses", "esses"), (G, G, G, G, G))

    def test_a_repeated_guess_letter_absent_from_the_answer_stays_grey(self):
        self.assertEqual(score_guess("crane", "zzzzz"), (X, X, X, X, X))

    def test_the_count_of_non_grey_marks_never_exceeds_the_answer_count(self):
        # Whatever the shape, 'e' cannot be marked more often than the answer
        # holds it. "abbey" holds one.
        marks = score_guess("abbey", "eerie")
        lit = [m for i, m in enumerate(marks) if "eerie"[i] == "e" and m is not X]
        self.assertEqual(len(lit), 1)


class ScoringOracleTest(unittest.TestCase):
    """An independent oracle: an obviously correct but slow implementation."""

    @staticmethod
    def oracle(answer, guess):
        pool = list(answer)
        marks = [X] * len(guess)
        for i in range(len(guess)):
            if guess[i] == answer[i]:
                marks[i] = G
                pool.remove(guess[i])
        for i in range(len(guess)):
            if marks[i] is G:
                continue
            if guess[i] in pool:
                marks[i] = Y
                pool.remove(guess[i])
        return tuple(marks)

    def test_agrees_with_the_oracle_over_a_small_alphabet(self):
        alphabet = "abc"
        checked = 0
        for answer in itertools.product(alphabet, repeat=4):
            for guess in itertools.product(alphabet, repeat=4):
                a = "".join(answer)
                g = "".join(guess)
                self.assertEqual(score_guess(a, g), self.oracle(a, g), (a, g))
                checked += 1
        self.assertEqual(checked, 81 * 81)


if __name__ == "__main__":
    unittest.main()
