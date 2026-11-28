"""The port: somewhere a count can be kept.

Rule 2: a port imports the domain only, for value types. It names *what* is
needed, never *how*. `Count` is re-exported so that an adapter needs to import
nothing but this module (rule 4: a secondary adapter imports ports only).
"""
from typing import Protocol

from {{name_snake}}.domain.count import Count

__all__ = ["Count", "CountStore"]


class CountStore(Protocol):
    def load(self) -> Count: ...

    def save(self, count: Count) -> None: ...
