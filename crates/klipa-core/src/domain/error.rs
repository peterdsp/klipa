use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("not found")]
    NotFound,
    #[error("invalid input: {0}")]
    Invalid(String),
}
