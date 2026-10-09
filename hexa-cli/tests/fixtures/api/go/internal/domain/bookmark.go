// Package domain holds the bookmark, as the API speaks it.
package domain

// BookmarkID is the identity of one bookmark. On the wire, a string.
type BookmarkID string

// Bookmark is a saved link.
type Bookmark struct {
	ID      BookmarkID `json:"id"`
	URL     string     `json:"url"`
	Title   string     `json:"title"`
	Tags    []string   `json:"tags"`
	SavedAt string     `json:"savedAt"`
	Note    *string    `json:"note,omitempty"`
	// Kept for ranking. Never sent.
	Rank int `json:"-"`
}

// NewBookmark is what a client sends to save a link.
type NewBookmark struct {
	URL   string   `json:"url"`
	Title string   `json:"title"`
	Tags  []string `json:"tags"`
}
