pub(crate) mod line_indexer;
pub(crate) mod iterator;
pub mod eventual_index;
pub mod index;

pub use iterator::LogLine;
pub use line_indexer::LineIndexer;
pub(crate) use iterator::{LineIndexerDataIterator, LineIndexerIterator};
