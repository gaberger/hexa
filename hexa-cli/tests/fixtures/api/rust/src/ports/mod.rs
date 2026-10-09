//! The driving port: what the outside world may ask this application to do.

pub use crate::domain::bookmark::{Bookmark as BookmarkValue, NewBookmark};

/// What can go wrong, as the caller sees it.
#[derive(Debug)]
pub enum ApiError {
    /// The caller sent something the rules refuse.
    /// @hexa:status 400
    Invalid(String),
    /// @hexa:status 404
    NotFound,
    /// Something failed underneath.
    Unavailable,
}

/// @hexa:api service=bookmarks version=1.0.0
pub trait BookmarkApi: Send + Sync {
    /// Save a link, or merge it into the one already saved under that URL.
    /// @hexa:api POST /bookmarks 201
    fn create(&self, req: NewBookmark) -> Result<BookmarkValue, ApiError>;

    /// Fetch one bookmark.
    /// @hexa:api GET /bookmarks/{id}
    fn get(&self, id: &str) -> Result<BookmarkValue, ApiError>;

    /// Every bookmark carrying a tag.
    /// @hexa:api GET /bookmarks
    fn list_by_tag(&self, tag: &str, cursor: Option<String>) -> Result<Vec<BookmarkValue>, ApiError>;

    /// Forget a bookmark.
    /// @hexa:api DELETE /bookmarks/{id}
    fn delete(&self, id: &str) -> Result<(), ApiError>;

    /// Not part of the API: no tag.
    fn stats(&self) -> usize;
}
