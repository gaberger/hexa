//! What the user asked the program to do.

/// One run of the program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// Play a whole game with no human, and print the machine contract.
    Demo { seed: u64 },
    /// Play against the seeded chooser at a terminal.
    Interactive { seed: u64 },
}

impl Request {
    /// The seed the chooser starts from.
    pub fn seed(self) -> u64 {
        match self {
            Request::Demo { seed } | Request::Interactive { seed } => seed,
        }
    }
}
