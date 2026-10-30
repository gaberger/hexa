//! Guides at hexa's known dead ends (ADR-2610301500).
//!
//! The table is the domain's (`domain::guide`). The verbs that reach a dead
//! end print its guide, and `hexa gap` reads the table, so it is part of
//! those adapters' contract, reached here.

pub use crate::domain::guide::{check_commands, command_words, find, fix_path, guides, pr_text, rank, Guide, Hit, Row};
