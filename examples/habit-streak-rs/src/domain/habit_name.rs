//! A habit's name, parsed (ADR-2610281223 §2).

/// Why a raw name is not a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    ControlCharacter,
}

/// A trimmed, non-empty name with no control character. Equal by `key()`.
#[derive(Debug, Clone)]
pub struct HabitName {
    display: String,
    key: String,
}

impl HabitName {
    pub fn parse(raw: &str) -> Result<HabitName, NameError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(NameError::Empty);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(NameError::ControlCharacter);
        }
        Ok(HabitName { display: trimmed.to_string(), key: trimmed.to_lowercase() })
    }

    /// The trimmed spelling.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// The spelling lowercased with `str::to_lowercase`.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl PartialEq for HabitName {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for HabitName {}
