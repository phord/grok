use log::trace;
use regex::Regex;
use std::ops::Range;

use crate::{indexer::sane_index::SaneIndex, LogLine};
use crate::indexer::waypoint::Position;

/**
 * Basic EventualIndex that accumulates matching line offsets. Can be used for search or filter, despite the name.
 *
 * self.index grows as we navigate around, but it only accumulates lines that match our SearchType. Thus this filter
 * eventually indexes all lines that match the search criteria.
 */

 #[derive(Debug)]
pub enum SearchType {
    Regex(Regex),
    Raw(String),
    Bookmark,
    None,
}

pub struct IndexFilter {
    f: SearchType,

    /// Filter in (true) or out (false)
    include: bool,

    /// Memoized index of matching lines
    index: SaneIndex,
}

#[inline]
fn is_match_type(line: &str, typ: &SearchType) -> bool {
    match typ {
        SearchType::Regex(re) => re.is_match(line),
        SearchType::Raw(s) => line.contains(s),
        SearchType::None => true,
        _ => { todo!("Unimplemented search type"); false},
    }
}

// Standalone helpers
fn trim_newline(line: &str) -> &str {
    // FIXME: Also remove \r?
    line.strip_suffix("\n").unwrap_or(line)
}

impl Default for IndexFilter {
    fn default() -> Self {
        Self::new(SearchType::None, true)
    }
}

impl IndexFilter {
    pub fn new(f: SearchType, include: bool) -> Self {
        IndexFilter {
            f,
            include,
            index: SaneIndex::new(),
        }
    }

    #[inline]
    fn is_match(&self, line: &str) -> bool {
        is_match_type(line, &self.f) ^ (!self.include)
    }

    // Evaluate a new line for inclusion in the index
    pub fn eval(&mut self, line: &LogLine) -> bool {
        self.is_match(trim_newline(line.line.as_str()))
    }

    // Resolve the gap at Position with the range as given, and the found logline, if any.
    pub fn insert(&mut self, pos: &Position, range: &Range<usize>, offsets: &[usize]) -> (Position, Position) {
        assert!(pos.is_unmapped());
        self.index.insert_at(pos, offsets, range)
    }

    // Step to the next indexed line or gap
    #[inline]
    pub fn next(&self, find: Position) -> Position {
        self.index.next(find)
    }

    #[inline]
    pub fn count_lines(&self) -> usize {
        self.index.count_lines()
    }
    #[inline]
    pub fn indexed_bytes(&self) -> usize {
        self.index.indexed_bytes()
    }

}
