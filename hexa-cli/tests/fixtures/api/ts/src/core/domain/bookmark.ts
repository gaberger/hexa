/** The identity of one bookmark. On the wire, a string. */
export type BookmarkId = string;

/** A saved link. */
export interface Bookmark {
  id: BookmarkId;
  url: string;
  title: string;
  tags: string[];
  savedAt: string;
  note?: string;
}

/** What a client sends to save a link. */
export interface NewBookmark {
  url: string;
  title: string;
  tags: string[];
}
