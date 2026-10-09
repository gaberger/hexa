/** The driving port: what the outside world may ask this application to do. */
import { Bookmark, NewBookmark } from '../domain/bookmark.js';

export { Bookmark, NewBookmark };

/**
 * @hexa:api service=bookmarks version=1.0.0
 * @hexa:status 400
 * @hexa:status 404
 */
export interface BookmarkApi {
  /**
   * Save a link, or merge it into the one already saved under that URL.
   * @hexa:api POST /bookmarks 201
   */
  create(req: NewBookmark): Promise<Bookmark>;

  /**
   * Fetch one bookmark.
   * @hexa:api GET /bookmarks/{id}
   */
  get(id: string): Promise<Bookmark>;

  /**
   * Every bookmark carrying a tag.
   * @hexa:api GET /bookmarks
   */
  listByTag(tag: string, cursor?: string): Promise<Bookmark[]>;

  /**
   * Forget a bookmark.
   * @hexa:api DELETE /bookmarks/{id}
   */
  delete(id: string): Promise<void>;

  /** Not part of the API: no tag. */
  stats(): number;
}
