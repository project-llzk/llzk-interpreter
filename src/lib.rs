//! Interpreter for a small LLZK subset.

mod dispatch;
mod eval;
mod state;
mod value;

pub use eval::Interpreter;
pub use state::{ConstraintRecord, ExecutionState};
pub use value::{ArrayInstance, Felt, IntValue, StructInstance, Value};

/// Interpreter error.
#[derive(Debug)]
pub enum Error {
    /// Underlying MLIR / LLZK API error.
    Llzk(String),
    /// Unsupported operation.
    UnsupportedOp(String),
    /// Malformed or unexpected IR shape.
    MalformedOp(String),
    /// Missing symbol.
    SymbolNotFound(String),
    /// Missing runtime value.
    MissingValue(String),
    /// Type mismatch.
    TypeError(String),
    /// Constraint failure.
    ConstraintFailed(String),
    /// Parse failure.
    ParseError(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llzk(msg)
            | Self::UnsupportedOp(msg)
            | Self::MalformedOp(msg)
            | Self::SymbolNotFound(msg)
            | Self::MissingValue(msg)
            | Self::TypeError(msg)
            | Self::ConstraintFailed(msg)
            | Self::ParseError(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<melior::Error> for Error {
    fn from(value: melior::Error) -> Self {
        Self::Llzk(value.to_string())
    }
}

impl From<llzk::error::Error> for Error {
    fn from(value: llzk::error::Error) -> Self {
        Self::Llzk(value.to_string())
    }
}
