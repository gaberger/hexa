"""A secondary adapter: the count kept in memory.

Rule 4: it imports the port only, never the domain and never another adapter.
Rule 5 is the other half: nothing imports it except the composition root.
"""
from {{name_snake}}.ports.store import Count, CountStore


class MemoryStore(CountStore):
    def __init__(self) -> None:
        self._count = Count()

    def load(self) -> Count:
        return self._count

    def save(self, count: Count) -> None:
        self._count = count
