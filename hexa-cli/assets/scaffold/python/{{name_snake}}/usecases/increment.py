"""The use case: increment the stored count by one.

Rule 3: a use case imports the domain and ports only; it is handed its store.
"""
from {{name_snake}}.domain.count import Count
from {{name_snake}}.ports.store import CountStore


def increment(store: CountStore) -> Count:
    count = store.load().incremented()
    store.save(count)
    return count
