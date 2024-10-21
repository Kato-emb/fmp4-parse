mod error;
mod segment;
mod writer;

pub type Result<T> = std::result::Result<T, Fmp4ParseError>;

pub use error::Fmp4ParseError;
pub use segment::{Chunk, InitialSegment, MediaSegment, Segment};
pub use writer::{FMp4Config, HybridMp4Writer};
