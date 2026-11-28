import unittest

from {{name_snake}}.composition_root import run


class IncrementTest(unittest.TestCase):
    def test_increment_counts_one(self) -> None:
        self.assertEqual(run().value, 1)


if __name__ == "__main__":
    unittest.main()
