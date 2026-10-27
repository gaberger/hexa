//! Which hexa verb a plain-English request names (ADR-2610271308). Pure: the
//! caller confirms and runs what is routed.

/// One routed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// For `Hexa`, the arguments after `hexa`; for `Shell`, the whole command.
    pub command: String,
    /// It changes or deletes something, so it needs confirmation even with `--yes`.
    pub destructive: bool,
    /// One line saying what it will do.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// A hexa verb: run as `hexa <command>`.
    Hexa(Route),
    /// A command that is not hexa's, such as `git status`.
    Shell(Route),
    /// Nothing in the table matches: say so, never guess.
    None,
}

/// The route for `text`, or `Routed::None`.
pub fn route(text: &str) -> Routed {
    let _ = text;
    unimplemented!("ADR-2610271308: built to its sealed spec")
}

/// Every hexa command the table can route to, so each can be checked against
/// the CLI.
pub fn hexa_commands() -> Vec<String> {
    unimplemented!("ADR-2610271308: built to its sealed spec")
}
