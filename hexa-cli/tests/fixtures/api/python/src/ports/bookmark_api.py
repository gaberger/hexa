from dataclasses import dataclass
from typing import Protocol


# The value types are declared beside the port, so this file is the whole
# contract.
@dataclass(frozen=True)
class Bookmark:
    id: str
    url: str
    title: str
    tags: list[str]
    savedAt: str
    note: str | None = None


@dataclass(frozen=True)
class NewBookmark:
    url: str
    title: str
    tags: list[str]


# @hexa:api service=bookmarks version=1.0.0
# @hexa:status 400 invalid
# @hexa:status 404 not found
class BookmarkApi(Protocol):
    # Save a link, or merge it into the one already saved under that URL.
    # @hexa:api POST /bookmarks 201
    def create(self, req: NewBookmark) -> Bookmark: ...

    # @hexa:api GET /bookmarks/{id}
    def get(self, id: str) -> Bookmark:
        """Fetch one bookmark."""
        ...

    # Every bookmark carrying a tag.
    # @hexa:api GET /bookmarks
    async def list_by_tag(self, tag: str, cursor: str | None = None) -> list[Bookmark]: ...

    # Forget a bookmark.
    # @hexa:api DELETE /bookmarks/{id}
    async def delete(self, id: str) -> None: ...

    # Not part of the API: no tag.
    def stats(self) -> int: ...
