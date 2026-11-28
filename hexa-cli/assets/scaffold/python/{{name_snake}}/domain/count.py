"""The domain: a count that only goes up.

Rule 1: the domain imports nothing outside the domain, so this module uses the
standard library's pure parts only.
"""
from dataclasses import dataclass


@dataclass(frozen=True)
class Count:
    value: int = 0

    def incremented(self) -> "Count":
        return Count(self.value + 1)
