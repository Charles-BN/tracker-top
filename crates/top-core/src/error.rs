use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// A required column is missing from the input header.
    MissingColumn(String),
    /// A cell could not be parsed (line number is 1-based, header = line 1).
    BadValue {
        line: usize,
        column: String,
        value: String,
    },
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::MissingColumn(c) => write!(f, "missing required column \"{c}\""),
            Error::BadValue {
                line,
                column,
                value,
            } => write!(f, "line {line}: cannot parse \"{value}\" in column \"{column}\""),
            Error::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
