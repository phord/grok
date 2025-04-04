use crate::config::Config;
use document::MergedLogs;
use indexed_file::{files, IndexedLog, LineIndexerDataIterator, Log};
use indexed_file::files::{Stream, ZstdLogFile};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;


fn get_files_from_cfg() -> Vec<Option<PathBuf>> {
    let cfg = Config::from_env().expect("Config should not error");
    log::trace!("Init config: {:?}", cfg);
    let mut files:Vec<Option<_>> = cfg.filename.iter().cloned().map(Some).collect();
    if files.is_empty() {
        files.push(None);
    };
    files
}

// Iterate using IndexedLog iterator, and poll for end of stream
#[allow(dead_code)]
pub fn cat_cmd() {
    let mut out = std::io::stdout();
    for file in get_files_from_cfg() {
        let file = files::new_text_file(file.as_ref()).expect("File failed to open");
        let mut file = Log::from(file);

        let mut start = 0;
        loop {
            let range = start..;
            for line in file.iter_lines_range(&range).filter(|line| line.offset >= range.start) {
                // stdout like this is almost twice as fast as print!("{line}");
                let _ = out.write(line.line.as_bytes()).expect("No errors");
                start = line.offset + 1;
            }
            if !file.is_open() {
                break
            }
            let timeout = std::time::Instant::now() + std::time::Duration::from_millis(100);
            file.poll(Some(timeout));
        }
    }
}


pub fn tac_cmd() {
    let mut logs = MergedLogs::new();
    for file in get_files_from_cfg() {
        let mut log = Log::open(file.as_ref()).unwrap();
        log.wait_for_end();
        logs.push(log);
    }
    // TODO: Print lines with colors
    for line in logs.iter_lines().rev() {
        print!("{line}");
    }
}

// Print last 10 lines from each file
pub fn tail_cmd() {
    // TODO: get from config
    let count = 10;

    for file in get_files_from_cfg() {
        let file = file.as_ref();
        let file = files::new_text_file(file).expect("File failed to open");
        let mut log = Log::from(file);

        let first_line = log
            .iter_lines()
            .rev()
            .take(count)
            .last()
            .unwrap();

        let range = first_line.offset..;
        for line in LineIndexerDataIterator::range(&mut log, &range) {
            print!("{line}");
        }
    }
}


use std::io::{self, Cursor, Read};

struct IteratorAsRead<I>
where
    I: Iterator,
{
    iter: I,
    cursor: Option<Cursor<I::Item>>,
}

impl<I> IteratorAsRead<I>
where
    I: Iterator,
{
    pub fn new<T>(iter: T) -> Self
    where
        T: IntoIterator<IntoIter = I, Item = I::Item>,
    {
        let mut iter = iter.into_iter();
        let cursor = iter.next().map(Cursor::new);
        IteratorAsRead { iter, cursor }
    }
}

impl<I> Read for IteratorAsRead<I>
where
    I: Iterator,
    Cursor<I::Item>: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while let Some(ref mut cursor) = self.cursor {
            let read = cursor.read(buf)?;
            if read > 0 {
                return Ok(read);
            }
            self.cursor = self.iter.next().map(Cursor::new);
        }
        Ok(0)
    }
}

// MergedLogs line iterator exits early on slow stdin because it doesn't wait for more data while the file is still open.
#[allow(dead_code)]
pub fn merged_cat_cmd() {
    let mut logs = MergedLogs::new();
    for file in get_files_from_cfg() {
        logs.push(Log::open(file.as_ref()).unwrap());
    }

    use std::io::Write;
    // locking stdout saves time vs. many calls to printf!() macro
    let stdout = std::io::stdout();
    let lock = stdout.lock();

    // Buffered writes approximately double the speed
    let mut out = BufWriter::new(lock);

    // TODO: Print lines with colors
    for line in logs.iter_lines() {
        write!(out, "{}", line).expect("stdout doesn't fail");
    }
}


// Copy from a merged log to stdout using io::copy and IteratorAsRead
// This also ends early when the stream underflows
// TEST: for i in {1..100} ; do echo $(for _ in {0..30} ; do printf "%5d" $i ; done); sleep 0.1 ; done | cargo run --release --bin cat
// Could implement Stream::wait() for MergedLogs to continue streaming while it's still open
#[allow(dead_code)]
pub fn copycat_merged_cmd() {
    let mut logs = MergedLogs::new();
    for file in get_files_from_cfg() {
        logs.push(Log::open(file.as_ref()).unwrap());
    }

    let mut src = IteratorAsRead::new(logs.iter_lines().map(|line| line.line));
    std::io::copy(&mut src, &mut std::io::stdout()).expect("We don't need error handling");
}


// Copy directly from our file to stdout using io::copy
#[allow(dead_code)]
pub fn copycat_cmd() {
    for file in get_files_from_cfg() {
        let mut file = files::new_text_file(file.as_ref()).expect("File failed to open");
        std::io::copy(&mut file, &mut std::io::stdout()).expect("We don't need error handling");
    }
}

// Iterate using BufRead::lines()
#[allow(dead_code)]
pub fn itercat_cmd() {
    let mut out = std::io::stdout();
    for file in get_files_from_cfg() {
        let file = files::new_text_file(file.as_ref()).expect("File failed to open");
        for line in file.lines() {
            let line = line.unwrap();
            let _ = out.write(line.as_bytes()).expect("No errors");
            let _ = out.write(b"\n").expect("No errors");
        }
    }
}

// Iterate using BufRead::lines() without my LogFile wrapper
#[allow(dead_code)]
pub fn brcat_cmd() {
    let mut out = std::io::stdout();
    for file in get_files_from_cfg() {
        let file = File::open(file.unwrap()).unwrap();
        let file = BufReader::new(file);
        for line in file.lines() {
            let line = line.unwrap();
            let _ = out.write(line.as_bytes()).expect("No errors");
            let _ = out.write(b"\n").expect("No errors");
        }
    }
}

// Reverse cat by iterating BufRead::lines().
// BufRead::lines() is not a double-ended iterator, so we have to make a copy in RAM first
#[allow(dead_code)]
pub fn rev_itercat_cmd() {
    let mut out = std::io::stdout();
    for file in get_files_from_cfg() {
        let file = files::new_text_file(file.as_ref()).expect("File failed to open");
        let lines = file.lines().map(|x| x.unwrap()).collect::<Vec<_>>();
        eprintln!("Read in {} lines", lines.len());
        for line in lines.iter().rev() {
            let _ = out.write(line.as_bytes()).expect("No errors");
            let _ = out.write(b"\n").expect("No errors");
        }
    }
}


#[allow(dead_code)]
pub fn bufcat_cmd() {
    let mut out = std::io::stdout();
    let mut buf:Vec<u8> = Vec::new();
    buf.reserve(10 * 1024);
    for file in get_files_from_cfg() {
        if let Ok(mut file) = ZstdLogFile::from_path(&file.unwrap()) {
            loop {
                let bytes = file.read_until(b'\n', &mut buf).expect("No errors");
                if bytes == 0 {
                    break
                }
                let _ = out.write(buf.as_slice()).expect("No errors");
            }
        }
    }
}


//#[test]
#[allow(dead_code)]
fn test_itercat() {
    let mut path = PathBuf::new();
    // path.push("/home/phord/git/mine/igrok/indexed_file/resources/test/core.log-2022040423.zst");
    path.push("/home/phord/git/mine/igrok/indexed_file/resources/test/core.log-2022040423");
    let mut out = std::io::stdout();
    let file = files::new_text_file(Some(&path)).expect("File failed to open");
    {
        for line in file.lines().map(|x| x.unwrap()).collect::<Vec<_>>().iter().rev() {
            let _ = out.write(line.as_bytes()).expect("No errors");
            let _ = out.write(b"\n").expect("No errors");
        }
    }
}
