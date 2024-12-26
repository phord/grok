// Wrapper to discover and iterate log lines from a LogFile while memoizing parsed line offsets

// Tests for LineIndexerDataIterator
#[cfg(test)]
mod logfile_data_iterator_tests {
    use std::collections::HashSet;

    use indexed_file::files::new_mock_file;
    use indexed_file::{Log, LineIndexerDataIterator};


    #[test]
    fn test_iterator() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut it = LineIndexerDataIterator::new(&mut file);
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);
        let mut prev = prev;
        assert_eq!(prev, 0);
        assert_eq!(line, patt);
        for i in it.take(lines - 1) {
            let (line, bol) = (i.line, i.offset);
            // println!("{prev} -> {bol}: {}", bol-prev);
            assert_eq!(bol - prev, patt_len);
            assert_eq!(line, patt);
            prev = bol;
        }
    }

    #[test]
    fn test_iterator_rev() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut it = LineIndexerDataIterator::new(&mut file).rev();
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);
        let mut prev = prev;

        assert_eq!(prev, lines * patt_len - patt_len);

        for i in it.take(lines - 2) {
            let (line, bol) = (i.line, i.offset);
            // println!("{bol} {prev}");
            assert_eq!(prev - bol, patt_len);
            assert_eq!(line, patt);
            prev = bol;
        }
    }

    #[test]
    fn test_iterator_rev_exhaust() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 3; //6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut it = LineIndexerDataIterator::new(&mut file).rev();
        let line = it.next().unwrap();
        let (_line, prev) = (line.line, line.offset);

        let mut prev = prev;

        assert_eq!(prev, lines * patt_len - patt_len);

        let mut count = 1;
        for i in it {
            let (line, bol) = (i.line, i.offset);
            // println!("{bol} {prev}");
            assert_eq!(prev - bol, patt_len);
            assert_eq!(line, patt);
            prev = bol;
            count += 1;
        }
        assert_eq!(count, lines);
    }

    #[test]
    fn test_iterator_exhaust() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;
        for _ in LineIndexerDataIterator::new(&mut file) {
            count += 1;
        }
        assert_eq!(count, lines);
    }

    #[test]
    fn test_iterator_exhaust_twice() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;
        for _ in LineIndexerDataIterator::new(&mut file) {
            count += 1;
        }
        assert_eq!(count, lines);

        let mut it = LineIndexerDataIterator::new(&mut file);
        // Iterate again and measure per-line and offsets
        let line = it.next().unwrap();
        let mut prev = line.offset;
        assert_eq!(prev, 0);
        for i in it.take(lines - 1) {
            let (line, bol) = (i.line, i.offset);
            assert_eq!(bol - prev, patt_len);
            assert_eq!(line, patt);
            prev = bol;
        }
    }


    #[test]
    fn test_iterator_exhaust_half_and_twice() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 6000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;
        for _ in LineIndexerDataIterator::new(&mut file).take(lines/2) {
            count += 1;
        }
        assert_eq!(count, lines/2);

        for _ in 0..2 {
            let mut it = LineIndexerDataIterator::new(&mut file);
            // Iterate again and measure per-line and offsets
            let line = it.next().unwrap();
            let mut prev = line.offset;
            assert_eq!(prev, 0);
            for i in it.take(lines - 1) {
                let (line, bol) = (i.line, i.offset);
                assert_eq!(bol - prev, patt_len);
                assert_eq!(line, patt);
                prev = bol;
            }
        }
    }


    #[test]
    fn test_iterator_from_offset_unindexed() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 100;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);

        // A few bytes before the middle of the file
        let range = patt_len * lines / 2 - patt_len / 2..;
        let mut it = LineIndexerDataIterator::range(&mut file, &range);

        // Iterate again and verify we get the expected number of lines
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        let mut count = 1;
        assert_eq!(prev, patt_len * (lines / 2 - 1));
        assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, lines / 2 + 1);
    }

    #[test]
    #[ignore]   // middle-out doesn't work on conforming iterators
    fn test_iterator_middle_out() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 1000;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;

        // A few bytes after the middle of the file
        todo!("duplicate iterator for reading in the other direction");
        let range = patt_len * lines / 2 - patt_len / 2..;
        let mut it = LineIndexerDataIterator::range(&mut file, &range);

        // Iterate forwards and backwards simultaneously
        let mut lineset = HashSet::new();
        loop {
            let mut done = true;
            if let Some(line) = it.next() {
                lineset.insert(line.offset);
                // We don't reach the end of the file
                assert!(line.offset < lines * patt_len);
                assert_eq!(line.line, patt);
                count += 1;
                done = false;
            }
            if let Some(line) = it.next_back() {
                lineset.insert(line.offset);
                assert_eq!(line.line, patt);
                count += 1;
                done = false;
            }
            if done {
                break;
            }
        }
        assert_eq!(lines, lineset.len());
        assert_eq!(count, lines);
    }

    #[test]
    fn test_iterator_from_offset_indexed() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 100;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;
        for _ in LineIndexerDataIterator::new(&mut file) {
            count += 1;
        }
        assert_eq!(count, lines);

        // range(start, end) should iterate over all lines whose lasted byte is >= start and first byte is < end
        // So this start should return the line just before the middle of the file.

        // A few bytes before the middle of the file
        let start = patt_len * lines / 2 - patt_len / 2;
let range=start..;
        let mut it = LineIndexerDataIterator::range(&mut file,  &range);

        // Iterate again and verify we get the expected number of lines
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        count = 1;
        assert_eq!(prev, patt_len * (lines / 2 - 1));
        assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, lines / 2 + 1);
    }

    #[test]
    fn test_iterator_from_offset_start() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 100;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let mut count = 0;
        let range = ..=0;
        for _ in LineIndexerDataIterator::range(&mut file, &range).rev() {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable before offset 0");

        let range = ..=1;
        for _ in LineIndexerDataIterator::range(&mut file, &range).rev() {
            count += 1;
        }
        assert_eq!(count, 1, "First line is reachable from offset 1");

        let range=0..;
        let mut it = LineIndexerDataIterator::range(&mut file,  &range);

        // Verify we see the first line
        let line = it.next().unwrap();
        let (line, prev) = (line.line, line.offset);

        count = 1;
        assert_eq!(prev, 0);
        assert_eq!(line, patt);

        for _ in it {
            count += 1;
        }
        assert_eq!(count, lines);
    }
    #[test]
    fn test_iterator_from_offset_end_of_file() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 100;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);
        let out_of_range = patt_len * lines;

        let mut count = 0;
        let range = out_of_range..;
        for _ in LineIndexerDataIterator::range(&mut file, &range) {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable after out-of-range");

        let range = ..out_of_range;
        for _ in LineIndexerDataIterator::range(&mut file, &range).rev() {
            count += 1;
        }
        assert_eq!(count, lines, "Whole file is reached from end");

    }

    #[test]
    fn test_iterator_from_offset_out_of_range() {
        let patt = "filler\n";
        let patt_len = patt.len();
        let lines = 100;
        let file = new_mock_file(patt, patt_len * lines, 100);
        let mut file = Log::from(file);

        let out_of_range = patt_len * lines + 2;

        let mut count = 0;
        let range = ..out_of_range;
        for _ in LineIndexerDataIterator::range(&mut file, &range).rev() {
            count += 1;
        }
        assert_eq!(count, lines, "All lines iterable before out-of-range");

        count = 0;
        let range = out_of_range..;
        for _ in LineIndexerDataIterator::range(&mut file, &range) {
            count += 1;
        }
        assert_eq!(count, 0, "No lines iterable after out-of-range");
     }
}