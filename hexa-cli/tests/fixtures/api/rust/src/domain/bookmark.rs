//! The bookmark, as the API speaks it.

use serde::{Deserialize, Serialize};

/// The identity of one bookmark. On the wire, a string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkId(pub String);

/// A saved link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: BookmarkId,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
    pub saved_at: String,
    pub note: Option<String>,
    /// Kept for ranking. Never sent.
    #[serde(skip)]
    pub rank: u32,
}

/// What a client sends to save a link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBookmark {
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}
