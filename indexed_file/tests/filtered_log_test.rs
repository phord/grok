

#[cfg(test)]
mod filtered_log_iterator_helper {
    use indexed_file::{FilteredLog, IndexedLog, LineIndexerDataIterator, Log};
    use indexed_file::files::{CursorLogFile, CursorUtil};

    pub(crate) struct Harness {
        pub(crate) patt_len: usize,
        pub(crate) lines: usize,
    }

    impl Harness {
        pub(crate) fn new(lines: usize) -> (Self, FilteredLog) {
            let patt_len = 9usize;
            let base = 10usize.pow(patt_len as u32 - 2);
            let buff = CursorLogFile::from_vec((base..base+lines).collect()).unwrap();
            let file = Log::from(buff);
            let file: FilteredLog = FilteredLog::new(Box::new(file));
            let s = Self {
                patt_len,
                lines,
            };
            (s, file)
        }


        // pub(crate) fn expected_line(&self, offset: usize, width: usize) -> &str {
        //     let offset = self.expected_bol(offset, width);
        //     let ofs = self.offset_into_line(offset);
        //     let width = self.expected_width(offset, width);
        //     &self.patt[ofs..ofs + width]
        // }

        pub(crate) fn default() -> (Self, FilteredLog) {
            Self::new(6000)
        }

        pub(crate) fn new_small(lines: usize) -> (Self, FilteredLog) {
            Self::new(lines)
        }
    }

    pub(crate) fn new<LOG: IndexedLog>(log: &mut LOG) -> LineIndexerDataIterator<LOG> {
        LineIndexerDataIterator::new(log)
    }

    pub(crate) fn new_from<LOG: IndexedLog>(log: &mut LOG, offset: usize) -> LineIndexerDataIterator<LOG> {
        LineIndexerDataIterator::new_from(log, offset)
    }
}


// Tests for filtered_log iterators
#[cfg(test)]
mod filtered_log_iterator_tests {
    use std::collections::HashSet;

    use crate::filtered_log_iterator_helper::{new, Harness};
    use indexed_file::index_filter::SearchType;
    use indexed_file::{LineIndexerDataIterator, Log};
    use regex::Regex;

    #[test]
    fn test_iterator() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let mut it = new(&mut file);
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);
        let mut prev = prev;
        assert_eq!(prev, 0);
        let mut count = 1;
        for i in it {
            count += 1;
            assert!(count <= harness.lines);
            let (_line, bol) = (i.line, i.offset);
            assert_eq!(bol - prev, harness.patt_len);
            prev = bol;
        }

        let it = new(&mut file);
        assert_eq!(it.count(), harness.lines);
    }

    #[test]
    fn test_iterator_subset() {
        let (harness, mut file) = Harness::default();
        // Match lines from 5,000 to 5,999
        file.search_regex("5...$").unwrap();

        let mut it = new(&mut file);
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);
        let mut prev = prev;
        assert_eq!(prev, 5000 * harness.patt_len);
        for i in it.take(harness.lines - 1) {
            let (_line, bol) = (i.line, i.offset);
            assert_eq!(bol - prev, harness.patt_len);
            prev = bol;
        }

        let it = new(&mut file);
        assert_eq!(it.count(), 1000);
    }

    #[test]
    fn test_iterator_no_match() {
        let (_harness, mut file) = Harness::default();
        // Match lines from 5,000 to 5,999
        file.search_regex("xyz").unwrap();

        let mut it = new(&mut file);
        assert!(it.next().is_none());

        let it = new(&mut file);
        assert_eq!(it.count(), 0);
    }

    #[test]
    fn test_iterator_rev() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let mut it = new(&mut file).rev();
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);
        let mut prev = prev;

        assert_eq!(prev, harness.lines * harness.patt_len - harness.patt_len);

        for i in it.take(harness.lines - 1) {
            let (_, bol) = (i.line, i.offset);
            // println!("{bol} {prev}");
            assert_eq!(prev - bol, harness.patt_len);
            prev = bol;
        }

        let it = new(&mut file);
        assert_eq!(it.count(), harness.lines);
    }


    #[test]
    fn test_iterator_gaps() {
        let (harness, mut file) = Harness::default();
        assert!(file.search_regex(r"0$").is_ok());

        let mut it = new(&mut file);
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);
        assert!(line.trim().ends_with("0"));
        let mut prev = prev;

        assert_eq!(prev, 0);

        let mut count = 1;
        for i in it.take(harness.lines - 1) {
            let (line, bol) = (i.line, i.offset);
            assert!(line.trim().ends_with("0"));
            assert_eq!(bol - prev, harness.patt_len * 10);
            prev = bol;
            count += 1;
        }
        assert_eq!(count, harness.lines / 10);

        let it = new(&mut file);
        assert_eq!(it.count(), harness.lines / 10);
    }


    #[test]
    fn test_iterator_rev_gaps() {
        let (harness, mut file) = Harness::default();
        assert!(file.search_regex(r"0$").is_ok());

        let mut it = new(&mut file).rev();
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);
        assert!(line.trim().ends_with("0"));
        let mut prev = prev;

        assert_eq!(prev, harness.lines * harness.patt_len - harness.patt_len * 10);

        let mut count = 1;
        for i in it.take(harness.lines - 1) {
            let (line, bol) = (i.line, i.offset);
            assert!(line.trim().ends_with("0"));
            assert_eq!(prev - bol, harness.patt_len * 10);
            prev = bol;
            count += 1;
        }
        assert_eq!(count, harness.lines / 10);

        let it = new(&mut file).rev();
        assert_eq!(it.count(), harness.lines / 10);
    }


    #[test]
    fn test_build_index() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();
        let mut it = new(&mut file);
        let line = it.next().unwrap();
        let prev = line.offset;
        let mut prev = prev;
        assert_eq!(prev, 0);
        for i in it.take(harness.lines - 1) {
            let bol = i.offset;
            assert_eq!(bol - prev, harness.patt_len);
            prev = bol;
        }
    }

    #[test]
    fn test_build_index_rev() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let mut it = new(&mut file).rev();
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);
        let mut prev = prev;

        assert_eq!(prev, harness.lines * harness.patt_len - harness.patt_len);

        for i in it.take(harness.lines - 2) {
            let (line, bol) = (i.line, i.offset);
            // println!("{bol} {prev}");
            assert_eq!(prev - bol, harness.patt_len);
            // assert_eq!(line, harness.patt);
            prev = bol;
        }
    }


    #[test]
    fn test_iterator_from_offset_unindexed() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        // A few bytes before the middle of the file
        let offset = harness.patt_len * harness.lines / 2 - harness.patt_len / 2;
        let mut it = LineIndexerDataIterator::new_from(&mut file, offset);

        // Iterate again and verify we get the expected number of lines
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        let mut count = 1;
        assert_eq!(prev, harness.patt_len * harness.lines / 2);
        // assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, harness.lines / 2);
    }

    #[test]
    fn test_iterator_middle_out() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        // A few bytes before the middle of the file
        let offset = harness.patt_len * harness.lines / 2 - harness.patt_len / 2;
        let mut it = LineIndexerDataIterator::new_from(&mut file, offset);

        // Iterate forwards and backwards simultaneously
        let mut count = 0;
        let mut lineset = HashSet::new();
        loop {
            let mut done = true;
            if let Some(line) = it.next() {
                lineset.insert(line.offset);
                // We don't reach the end of the file
                assert!(line.offset < harness.lines * harness.patt_len);
                // assert_eq!(line.line, patt);
                count += 1;
                done = false;
            }
            if let Some(line) = it.next_back() {
                lineset.insert(line.offset);
                // assert_eq!(line.line, patt);
                count += 1;
                done = false;
            }
            if done {
                break;
            }
        }
        assert_eq!(harness.lines, lineset.len());
        assert_eq!(count, harness.lines);
    }

    #[test]
    fn test_iterator_from_offset_indexed() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        // Iterate whole file (indexed)
        let mut count = 0;
        for _ in LineIndexerDataIterator::new(&mut file) {
            count += 1;
        }
        assert_eq!(count, harness.lines);

        // A few bytes before the middle of the file
        let offset = harness.patt_len * harness.lines / 2 - harness.patt_len / 2;
        let mut it = LineIndexerDataIterator::new_from(&mut file, offset);

        // Iterate again and verify we get the expected number of lines
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        count = 1;
        assert_eq!(prev, harness.patt_len * harness.lines / 2);
        // assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, harness.lines / 2);
    }

    #[test]
    fn test_iterator_from_offset_start() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let mut count = 0;
        for _ in LineIndexerDataIterator::new_from(&mut file, 0).rev() {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable before offset 0");

        for _ in LineIndexerDataIterator::new_from(&mut file, 1).rev() {
            count += 1;
        }
        assert_eq!(count, 1, "First line is reachable from offset 1");

        let mut it = LineIndexerDataIterator::new_from(&mut file, 0);

        // Verify we see the first line
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        count = 1;
        assert_eq!(prev, 0);
        // assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, harness.lines);
    }
    #[test]
    fn test_iterator_from_offset_end_of_file() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let out_of_range = harness.patt_len * harness.lines;

        let mut count = 0;
        for _ in LineIndexerDataIterator::new_from(&mut file, out_of_range) {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable after out-of-range");

        for line in LineIndexerDataIterator::new_from(&mut file, out_of_range).rev() {
            count += 1;
        }
        assert_eq!(count, harness.lines, "Whole file is reached from end");

    }

    #[test]
    fn test_iterator_from_offset_out_of_range() {
        let (harness, mut file) = Harness::default();
        file.search_regex("000").unwrap();

        let out_of_range = harness.patt_len * harness.lines + 2;

        let mut count = 0;
        for _ in LineIndexerDataIterator::new_from(&mut file, out_of_range).rev() {
            count += 1;
        }
        assert_eq!(count, harness.lines, "All lines iterable before out-of-range");

        count = 0;
        for _ in LineIndexerDataIterator::new_from(&mut file, out_of_range) {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable after out-of-range");
     }
}