use std::io::BufRead;

use super::waypoint::{Position, VirtualPosition, Waypoint};


/// SaneIndex
/// Holds a map of the explored regions of the file.
///      0: Hello, world\n
///     13: \n
///     14: This is a test.\n
///     30: This is only a test.\n
///     51: \n
///     52: End of message\n
///     67:
///
/// This file has 67 bytes.
/// Initially the file is unmapped:     [ Unmapped(0..IMAX) ]
///
/// -> When we read the first line, we learn the offset of the second one. Notice unmapped still includes the start of the 2nd line.
/// We read the first line and map it:  [ Mapped(0), Mapped(13), Unmapped(13..IMAX) ]
///
/// -> When we read the last line, we leave an umapped region at the end in case the file grows later.
/// We read the last line and map it:   [ Mapped(0), Mapped(13), Unmapped(13..51), Mapped(52), Unmapped(67..IMAX)]
/// We read the second line and map it: [ Mapped(0), Mapped(13), Mapped(14), Unmapped(14..51), Mapped(52), Unmapped(67..IMAX) ]
/// Finally we scan the middle region:  [ Mapped(0), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Unmapped(67..IMAX) ]
///
/// Suppose we mapped the middle section of the file first.
/// Initially the file is unmapped:     [ Unmapped(0..IMAX) ]
/// We scan bytes 10 to 39:             [ Unmapped(0..10), Mapped(13), Mapped(14), Mapped(30), Unmapped(40..IMAX) ]
///
/// Note we always assume there is a line at Mapped(0).  But it may not be inserted in every index.

/// Updated to use a splitvec-style implementation when growing in the middle.
/// Each internal vector either has a single Unmapped(range) or more Mapped(offset) values.


const IMAX:usize = usize::MAX;
type Range = std::ops::Range<usize>;

type IndexVec = Vec<Vec<Waypoint>>;
pub type IndexIndex = (usize, usize);

pub struct SaneIndex {
    pub(crate) index: IndexVec,
}

impl Default for SaneIndex {
    fn default() -> Self {
        SaneIndex {
            index: vec![vec![Waypoint::Unmapped(0..IMAX)]],
        }
    }
}

impl SaneIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_prev(&self, idx: IndexIndex) -> Option<IndexIndex> {
        let (i, j) = idx;
        if j > 0 {
            Some((i, j - 1))
        } else if i > 0 {
            Some((i - 1, self.index[i - 1].len() - 1))
        } else {
            None
        }
    }

    pub fn index_next(&self, idx: IndexIndex) -> Option<IndexIndex> {
        let (i, j) = idx;
        if j + 1 < self.index[i].len() {
            Some((i, j + 1))
        } else if i + 1 < self.index.len() {
            Some((i + 1, 0))
        } else {
            None
        }
    }

    pub fn index_valid(&self, idx: IndexIndex) -> bool {
        let (i, j) = idx;
        i < self.index.len() && j < self.index[i].len()
    }

    pub fn value(&self, idx: IndexIndex) -> &Waypoint {
        let (i, j) = idx;
        &self.index[i][j]
    }

    /// Find the index holding the given offset, or where it would be inserted if none found.
    pub(crate) fn search(&self, offset: usize) -> IndexIndex {
        let target = &Waypoint::Mapped(offset);
        let find = self.index.binary_search_by_key(&target, |v| v.first().unwrap());
        let ndx  = match find {
            // Found the matching index
            Ok(i) => (i, 0),
            // Found where the index should be inserted
            Err(i) => {
                let i = i.saturating_sub(1);
                match self.index[i].binary_search(target) {
                    Ok(j) => (i, j),
                    Err(j) => {
                        if j == self.index[i].len() {
                            (i + 1, 0)
                        } else {
                            (i, j)
                        }
                    },
                }
            },
        };

        if let Some(prev) = self.index_prev(ndx) {
            if self.value(prev).contains(offset) {
                return prev;
            }
        }
        if self.index_valid(ndx) && offset > self.value(ndx).cmp_offset() {
            if let Some(next) = self.index_next(ndx) {
                return next;
            }
        }
        ndx
    }

    pub(crate) fn next(&self, pos: Position) -> Position {
        let mut pos = pos;
        pos.next(self);
        pos
    }

    pub(crate) fn next_back(&self, pos: Position) -> Position {
        let mut pos = pos;
        pos.next_back(self);
        pos
    }

    /// Find the Unmapped region that contains the gap and split it;
    /// return the index of the row that can be overwritten
    fn resolve_gap(&mut self, gap: Range) -> usize {
        let mut ndx = self.search(gap.start);
        if self.value(ndx).is_mapped() {
            if let Some(next) = self.index_next(ndx) {
                ndx = next;
            }
        } else if let Some(prev) = self.index_prev(ndx) {
            if self.value(prev).contains(gap.start) {
                ndx = prev;
            }
        }
        assert!(self.index_valid(ndx));

        let unmapped = &self.value(ndx);
        assert!(!unmapped.is_mapped());
        assert!(unmapped.end_offset() >= gap.end);
        assert!(unmapped.cmp_offset() <= gap.start);

        let (mut i, j) = ndx;
        assert!(j == 0, "unmapped regions should be in their own vector");
        assert!(self.index[i].len() == 1, "unmapped regions should be in their own vector");

        let (left, middle) = unmapped.split_at(gap.start);
        let (_, right) = middle.unwrap().split_at(gap.end);
        if let Some(left) = left {
            self.index.insert(i, vec![left]);
            i += 1;
        }
        if let Some(right) = right {
            self.index.insert(i + 1, vec![right]);
        }
        i
    }

    pub fn insert(&mut self, offsets: &[usize], range: Range) {
        // Remove gaps that covered the region
        let i = self.resolve_gap(range.clone());

        assert!(self.index[i].len() == 1, "unmapped regions should be in their own vector");
        assert!(!self.index[i][0].is_mapped());
        if offsets.is_empty() {
            self.index.remove(i);
        } else {
            self.index[i] = offsets.iter()
                .map(|offset| {
                    assert!(range.contains(offset) || range.end == *offset);
                    Waypoint::Mapped(*offset)
                }).collect();
        }
    }

    // Parse lines from a BufRead
    pub fn parse_bufread<R: BufRead>(&mut self, source: &mut R, range: &Range) -> std::io::Result<usize> {
        /* We want to do this, except it takes ownership of the source:
            let mut pos = offset;
            let newlines = source.lines()
                .map(|x| { pos += x.len() + 1; pos });
            self.line_offsets.extend(newlines);
            */
        let mut pos = range.start;
        let end = range.end;
        while pos < end {
            let bytes =
                match source.fill_buf() {
                    Ok(buf) => {
                        if buf.is_empty() {
                            break       // EOF
                        }
                        let len = buf.len().min(end - pos);
                        self.parse_chunk(pos, &buf[..len]);
                        len
                    },
                    Err(e) => {
                        return std::io::Result::Err(e)
                    },
                };
            pos += bytes;
            source.consume(bytes);
        }
        Ok(pos - range.start)
    }

    pub fn parse_chunk(&mut self, offset: usize, chunk: &[u8]) {
        let mut offsets: Vec<usize> = chunk.iter().enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(i, _)| offset + i + 1)
            .collect();
        if offset == 0 {
            offsets.insert(0, 0);
        }
        self.insert(&offsets, offset..offset + chunk.len());
    }

    pub(crate) fn iter(&self) -> SaneIter {
        SaneIter::new(self)
    }
}

pub struct SaneIter<'a> {
    index: &'a SaneIndex,
    pos: Position,
}

impl<'a> SaneIter<'a> {
    fn new(index: &'a SaneIndex) -> Self {
        SaneIter {
            pos: Position::Virtual(VirtualPosition::Start),
            index,
        }
    }
}

impl<'a> Iterator for SaneIter<'a> {
    type Item = Waypoint;

    fn next(&mut self) -> Option<Self::Item> {
        match self.index.next(self.pos.clone()) {
            Position::Existing(i, target, waypoint) => {
                self.pos = Position::Existing(i, target, waypoint.clone());
                Some(waypoint)
            },
            _ => {
                self.pos = Position::Virtual(VirtualPosition::Invalid);
                None
            },
        }
    }
}


#[test]
fn sane_index_basic() {
    let mut index = SaneIndex::new();
    index.insert(&[0], 0..13);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Unmapped(13..IMAX)]);
    index.insert(&[13], 13..14);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Mapped(13), Waypoint::Unmapped(14..IMAX)]);
    index.insert(&[14], 14..30);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Mapped(13), Waypoint::Mapped(14), Waypoint::Unmapped(30..IMAX)]);
    index.insert(&[30], 30..51);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Mapped(13), Waypoint::Mapped(14), Waypoint::Mapped(30), Waypoint::Unmapped(51..IMAX)]);
    index.insert(&[51], 51..52);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Mapped(13), Waypoint::Mapped(14), Waypoint::Mapped(30), Waypoint::Mapped(51), Waypoint::Unmapped(52..IMAX)]);
    index.insert(&[], 52..67);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(0), Waypoint::Mapped(13), Waypoint::Mapped(14), Waypoint::Mapped(30), Waypoint::Mapped(51), Waypoint::Unmapped(67..IMAX)]);
    assert_eq!(index.index.len(), 6);
}

#[test]
fn sane_index_basic_rev() {
    let mut index = SaneIndex::new();
    index.insert(&[], 52..67);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Unmapped(0..52), Waypoint::Unmapped(67..IMAX)]);
    index.insert(&[13], 13..14);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Unmapped(0..13), Waypoint::Mapped(13), Waypoint::Unmapped(14..52), Waypoint::Unmapped(67..IMAX)]);
    index.insert(&[], 0..13);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(13), Waypoint::Unmapped(14..52), Waypoint::Unmapped(67..IMAX)]);
    index.insert(&[14], 14..30);
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Waypoint::Mapped(13), Waypoint::Mapped(14), Waypoint::Unmapped(30..52), Waypoint::Unmapped(67..IMAX)]);
}


#[test]
fn sane_index_parse_basic() {
    use Waypoint::*;
    let mut index = SaneIndex::new();
    let file = "Hello, world\n\nThis is a test.\nThis is only a test.\n\nEnd of message\n";
    index.parse_chunk(0, file.as_bytes());
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Mapped(0), Mapped(13), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
}

#[test]
fn sane_index_parse_chunks() {
    use Waypoint::*;
    let mut index = SaneIndex::new();
    let file = "Hello, world\n\nThis is a test.\nThis is only a test.\n\nEnd of message\n";
    let start = 35;
    index.parse_chunk(start, file[start..].as_bytes());
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Unmapped(0..start), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
    index.parse_chunk(0, file[..start].as_bytes());
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Mapped(0), Mapped(13), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
}

#[test]
fn sane_index_parse_chunks_random_bytes() {
    use Waypoint::*;
    use rand::thread_rng;
    use rand::seq::SliceRandom;

    let mut index = SaneIndex::new();
    let file = "Hello, world\n\nThis is a test.\nThis is only a test.\n\nEnd of message\n";
    let mut rando:Vec<usize> = (0..=66).collect::<Vec<_>>();
    rando.shuffle(&mut thread_rng());
    for i in rando {
        index.parse_chunk(i, file[i..i+1].as_bytes());
    }
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Mapped(0), Mapped(13), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
}


#[test]
fn sane_index_parse_chunks_random_chunks() {
    use Waypoint::*;
    use rand::thread_rng;
    use rand::seq::SliceRandom;

    let mut index = SaneIndex::new();
    let file = "Hello, world\n\nThis is a test.\nThis is only a test.\n\nEnd of message\n";
    let mut rando:Vec<usize> = (1..=66).collect::<Vec<_>>();
    rando.shuffle(&mut thread_rng());
    let mut start = 0;

    // Collect 1/3 of the byte offsets from the file.
    let mut cuts:Vec<&usize> = rando.iter().take(rando.len()/3).collect();

    // Always ensure that the last byte is included.
    cuts.push(&67);
    cuts.sort();
    let mut cuts = cuts.iter().map(|i| { let s = start; start = **i; s..**i }).collect::<Vec<_>>();

    // Resolve the ranges in random order
    cuts.shuffle(&mut thread_rng());
    for i in cuts {
        index.parse_chunk(i.start, file[i].as_bytes());
    }
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Mapped(0), Mapped(13), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
}

#[test]
fn sane_index_full_bufread() {
    use Waypoint::*;

    let file = b"Hello, world\n\nThis is a test.\nThis is only a test.\n\nEnd of message\n";
    let mut cursor = std::io::Cursor::new(file);

    let mut index = SaneIndex::new();

    index.parse_bufread(&mut cursor, &(0..100)).unwrap();
    assert_eq!(index.iter().collect::<Vec<_>>(), vec![Mapped(0), Mapped(13), Mapped(14), Mapped(30), Mapped(51), Mapped(52), Mapped(67), Unmapped(67..IMAX)]);
}
