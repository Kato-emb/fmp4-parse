use thiserror::Error;

#[derive(Debug, Error)]
pub enum Fmp4ParseError {
    #[error("{0}")]
    IoError(#[from] std::io::Error),
    #[error("{0}")]
    Mp4Error(#[from] mp4::Error),
    #[error("{0}")]
    InvalidFormat(&'static str),
}
