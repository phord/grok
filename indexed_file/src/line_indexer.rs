// Structs to index lines in a file
// TODO: Cleanup - This is a clone of indexer (LogFile) that doesn't parse out words and numbers.  It only parses lines.
//       Needs to be allowed to run in the background better, in a way that Rust can accept.

use std::path::PathBuf;

use std::fs::File;
use std::fmt;
use mapr::{MmapOptions, Mmap};
use crossbeam::scope;
use crossbeam_channel::{bounded, unbounded};

struct Index {
    line_offsets: Vec<usize>,
}

impl Index {
    fn new() -> Index {
        Index {
            line_offsets: Vec::new(),
        }
    }

    fn bytes(&self) -> usize {
        *self.line_offsets.last().unwrap_or(&0)
    }

    fn lines(&self) -> usize {
        self.line_offsets.len()
    }

    // Accumulate the map of line offsets
    // Parse buffer starting at `offset` and stopping at the first eol after end_target
    // Skip the first line unless offset is zero
    // size is the bytes we must process. After that is overlap with the next buffer.
    fn parse(&mut self, data: &[u8], offset: usize, size: usize) -> usize {
        let bytes = data.len();
        let has_final_eol = data.last().unwrap() == &b'\n';
        let mut cnt = offset;

        let mut pos = 0;
        let max_pos = if has_final_eol { bytes } else { bytes + 1 };

        // Skip the first line if offset is not zero
        if offset > 0 {
            for c in data {
                pos += 1;
                if c == &b'\n' {
                    cnt = offset + pos;
                    break;
                }
            }
        }

        loop {  // for pos in 0..bytes { //for c in mmap.as_ref() {
            if pos >= max_pos {
                break;
            }
            let c = if pos < bytes { data[pos] } else { b'\n' };
            if c == b'\n' {
                cnt = offset + pos + 1;
                self.line_offsets.push(offset + pos + 1);
                if pos >= size {
                    break;
                }
            }
            pos += 1;
        }
        cnt
    }
}

struct EventualIndex {
    indexes: Vec<Index>,
    line_offsets: Vec<usize>,
}

impl EventualIndex {
    fn new() -> EventualIndex {
        EventualIndex {
            indexes: Vec::new(),
            line_offsets: Vec::new(),
        }
    }

    fn merge(&mut self, other: Index) {
        // merge lazily
        self.indexes.push(other);
    }

    fn finalize(&mut self) {
        self.indexes.sort_by_key(|index| *index.line_offsets.get(0).unwrap_or(&(1usize << 20)));

        // TODO: self.line_offsets is duplicate info; better to move from indexes or to always lookup from indexes
        for index in self.indexes.iter() {
            self.line_offsets.extend_from_slice(&index.line_offsets);
        }
    }

    fn line_offset(&self, line_number: usize) -> Option<usize> {
        if line_number >= self.line_offsets.len() {
            return None;
        }
        Some(self.line_offsets[line_number])
    }

    fn bytes(&self) -> usize {
        self.indexes.iter().fold(0, |a, v| std::cmp::max(v.bytes(), a))
    }

    fn lines(&self) -> usize {
        self.indexes.iter().fold(0, |a, v| a + v.lines())
    }

}


pub struct LogFileLines {
    // pub file_path: PathBuf,
    mmap: Mmap,
    index: EventualIndex,
}

impl fmt::Debug for LogFileLines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogFileLines")
         .field("bytes", &self.count_bytes())
         .field("lines", &self.count_lines())
         .finish()
    }
}

impl LogFileLines {
    // FIXME: Is there a way to mark this for tests only?
    pub fn test_new(input_file: Option<PathBuf>, chunk_size: usize, max_line_length: usize) -> std::io::Result<LogFileLines> {
        let mut file = LogFileLines::open(input_file)?;
        file.index_file(chunk_size, max_line_length);
        Ok(file)
    }
}

use std::io::{Error, ErrorKind};
impl LogFileLines {

    fn open(input_file: Option<PathBuf>) -> std::io::Result<LogFileLines> {

        let file = if let Some(file_path) = input_file {
            // Must have a filename as input.
            let file = File::open(file_path)?;
            Some(file)
        } else {

            // Print error.
            eprintln!("Expected '<input>' or input over stdin.");
            return Err(Error::new(ErrorKind::Other, "Expected a filename or stdin"));
        };

        let mmap = unsafe { MmapOptions::new().map(&file.unwrap()) };
        let mmap = mmap.expect("Could not mmap file.");

        let file = LogFileLines {
            // file_path: input_file.unwrap(),
            mmap,
            index: EventualIndex::new(),
        };

        Ok(file)
    }

    pub fn new(input_file: Option<PathBuf>) -> std::io::Result<LogFileLines> {
        let chunk_size = 1024 * 1024 * 1;
        let max_line_length: usize = 64 * 1024;

        let mut file = LogFileLines::open(input_file)?;
        file.index_file(chunk_size, max_line_length);
        Ok(file)
    }

    fn index_file(&mut self, chunk_size: usize, max_line_length: usize) {

        let bytes = self.mmap.len();
        let mut pos = 0;

        // TODO: Since lazy merge is free, kick off the threads here and keep them running. Then any readers
        // can collect results and merge them to get completed progress in real-time. This also give us a
        // chance to add a stop-signal so we can exit early.

        // Finalize needs to adapt, and this loop needs to run in its own thread.
        // In the future this mechanism can serve to read like tail -f or to read from stdin.

        let (tx, rx):(crossbeam_channel::Sender<Index>, crossbeam_channel::Receiver<_>) = unbounded();
        // Limit threadpool of parsers by relying on sender queue length
        let (sender, receiver) = bounded(6); // inexplicably, 6 threads is ideal according to empirical evidence on my 8-core machine

        scope(|scope| {
            // get indexes in chunks in threads
            while pos < bytes {
                let end = std::cmp::min(pos + chunk_size, bytes);
                let overflow = std::cmp::min(bytes, end + max_line_length);

                // Count parser threads
                sender.send(true).unwrap();

                // Send the buffer to the parsers
                let buffer = &self.mmap[pos..overflow];

                let tx = tx.clone();
                let receiver = receiver.clone();
                let start = pos;
                scope.spawn(move |_| {
                    let mut index = Index::new();
                    index.parse(&buffer, start, end - start);
                    tx.send(index).unwrap();
                    receiver.recv().unwrap();
                });
                pos = end;
            }

            // We don't need our own handle for this channel
            drop(tx);

            // Wait for results and merge them in
            while let Ok(index) = rx.recv() {
                self.index.merge(index);
            }
        }).unwrap();

        // Partially coalesce merged info
        self.index.finalize();
    }

    fn count_bytes(&self) -> usize {
        self.index.bytes()
    }

    pub fn count_lines(&self) -> usize {
        self.index.lines()
    }

    fn line_offset(&self, line_number: usize) -> Option<usize> {
        if line_number == 0 {
            Some(0)
        } else {
            self.index.line_offset(line_number - 1)
        }
    }

    pub fn readline(&self, line_number: usize) -> Option<&str> {
        let start = self.line_offset(line_number);
        let end = self.line_offset(line_number + 1);
        if let (Some(start), Some(end)) = (start, end) {
            self.readline_fixed(start, end)
        } else {
            None
        }
    }

    pub fn readline_fixed(&self, start: usize, end: usize) -> Option<&str> {
        if end < self.mmap.len() {
            assert!(end > start);
            // FIXME: Handle unwrap error
            // FIXME: Handle CR+LF endings
            Some(std::str::from_utf8(&self.mmap[start..end-1]).unwrap())
        } else {
            None
        }
    }

    pub fn iter_offsets(&self) -> impl Iterator<Item = (&usize, &usize)> + '_ {
        let starts = std::iter::once(&0usize).chain(self.index.line_offsets.iter());
        let ends = self.index.line_offsets.iter();
        let line_range = starts.zip(ends);
        line_range
    }

}
