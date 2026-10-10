// Package ports holds the driving port: what the outside world may ask.
package ports

import (
	"context"

	"bookmarks/internal/domain"
)

type Bookmark = domain.Bookmark
type NewBookmark = domain.NewBookmark

// BookmarkAPI is what the outside world may ask this application to do.
//
// @hexa:api service=bookmarks version=1.0.0
// @hexa:status 400
// @hexa:status 404
type BookmarkAPI interface {
	// Save a link, or merge it into the one already saved under that URL.
	// @hexa:api POST /bookmarks 201
	Create(ctx context.Context, req NewBookmark) (Bookmark, error)

	// Fetch one bookmark.
	// @hexa:api GET /bookmarks/{id}
	Get(ctx context.Context, id string) (Bookmark, error)

	// Every bookmark carrying a tag.
	// @hexa:api GET /bookmarks
	ListByTag(ctx context.Context, tag string, cursor *string) ([]Bookmark, error)

	// Forget a bookmark.
	// @hexa:api DELETE /bookmarks/{id}
	Delete(ctx context.Context, id string) error

	// Not part of the API: no tag.
	Stats() int
}
