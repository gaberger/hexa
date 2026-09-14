//! The command line. It never panics and never reads past the end of the list.

use crate::ports::Request;

/// The one usage line.
pub const USAGE: &str = "usage: connect-four [--demo] [--seed <n>]";

/// Every way the command line can be wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UsageError {
    DemoNeedsSeed,
    SeedNeedsValue,
    SeedNotDecimal,
    Unknown,
}

impl UsageError {
    /// The single line this error prints to stderr.
    pub fn message(self) -> &'static str {
        match self {
            UsageError::DemoNeedsSeed => "--demo needs --seed <n>",
            UsageError::SeedNeedsValue => "--seed needs a value",
            UsageError::SeedNotDecimal => "--seed needs a decimal number",
            UsageError::Unknown => USAGE,
        }
    }
}

/// Read a seed. Decimal digits only: no sign, no `0x`, no spaces, and it must
/// fit in 64 bits.
fn read_seed(text: &str) -> Result<u64, UsageError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(UsageError::SeedNotDecimal);
    }
    text.parse::<u64>().map_err(|_| UsageError::SeedNotDecimal)
}

/// Turn the arguments into a request. The program name is already removed.
pub fn parse(args: &[String]) -> Result<Request, UsageError> {
    let mut demo = false;
    let mut seed = 1_u64;
    let mut seed_given = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--demo" => {
                demo = true;
                index += 1;
            }
            "--seed" => {
                let value = args.get(index + 1).ok_or(UsageError::SeedNeedsValue)?;
                seed = read_seed(value)?;
                seed_given = true;
                index += 2;
            }
            _ => return Err(UsageError::Unknown),
        }
    }

    if demo && !seed_given {
        return Err(UsageError::DemoNeedsSeed);
    }
    if demo {
        Ok(Request::Demo { seed })
    } else {
        Ok(Request::Interactive { seed })
    }
}
