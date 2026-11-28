"""The composition root: the one place that names an adapter.

Rule 6: only a composition root imports adapters. It wires the store to the
use case, and everything else asks for the port.
"""
from {{name_snake}}.adapters.secondary.memory import MemoryStore
from {{name_snake}}.domain.count import Count
from {{name_snake}}.usecases.increment import increment


def run() -> Count:
    return increment(MemoryStore())
