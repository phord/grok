use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::path::PathBuf;
/**
 * CachedStreamReader is a non-blocking stream reader that implements Read, BufRead and Seek. It
 * supports Stdin from a terminal or a redirect, and pipes. A better name might be UnboundedReadBuffer because that
 * is how it functions internally to provide Seek and non-blocking Read.
 *
 * Random seeks are supported by keeping a copy of all the data ever received from stdin in memory. This is possibly
 * wasteful on some systems that already cache the stdin data somewhere, but it can't be helped in any portable way.
 *
 * It is non-blocking because when we try to read past the end of the data, we can read from our buffer instead
 * of from the stdin file handle.
 *
 * Data is spooled into our buffer from a listener thread and results are posted to a mpsc::sync_channel. Data
 * is read using read_line for portability. We could read bytes, but while leaving stdin in blocking mode, we
 * can't reliably read partial lines except by reading a byte at a time.
 *
 * To prevent runaway source pipes from filling all of RAM needlessly, we use a limit in a bounded channel of
 * lookahead_count lines to read ahead and we only pull from the queue if the caller wants to read near the end
 * of our currently loaded buffered data. Well-behaved apps will respond to this backpressure to avoid sending
 * more data as well, thus throttling the whole pipeline if needed.
 */

use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::TryRecvError;
use std::thread;

const QUEUE_SIZE:usize = 100;
const READ_THRESHOLD:usize = 10240;

pub trait Stream {
    fn len(&self) -> usize;
    // Wait on any data at all; Returns true if file is still open
    fn wait(&mut self) -> bool;
}

pub struct CachedStreamReader {
    buffer: Vec<u8>,
    rx: Option<Receiver<Vec<u8>>>,
    pos:u64,
}

impl CachedStreamReader {
    pub fn new(pipe: Option<PathBuf>) -> Self {
        let base = Self {
            rx: None,
            buffer: Vec::default(),
            pos: 0,
        };

        if let Some(pipe) = pipe {
            Self {
                rx: Some(Self::reader(Some(BufReader::new(File::open(pipe).expect("File exists"))))),
                ..base
            }
        } else {
            Self {
                rx: Some(Self::reader(None)),
                ..base
            }
        }
    }

    pub fn is_eof(&self) -> bool {
        !self.rx.is_some()
    }

    pub fn fill_buffer(&mut self, pos: usize) {
        // TODO: Merge this and wait(); always wait for anything at pos
        if pos + READ_THRESHOLD > self.len() {
            if let Some(rx) = &self.rx {
                loop {
                    match rx.try_recv() {
                        Ok(mut data) => self.buffer.append(&mut data),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            self.rx = None;
                            break;
                        },
                    }
                }
            }
        }
    }

    fn reader(mut pipe: Option<BufReader<File>>) -> Receiver<Vec<u8>>
    {
        // Use a bounded channel to prevent stdin from running away from us
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE_SIZE);
        let mut buffer = String::new();
        thread::spawn(move || loop {
            buffer.clear();
            // TODO: Read into a Vec<u8> and avoid utf8-validation of the data
            // TODO: Handle data with no line-feeds
            let line = match &mut pipe {
                Some(file) => file.read_line(&mut buffer),
                None => std::io::stdin().read_line(&mut buffer),
            };
            match line {
                Ok(0) => break,  // EOF
                Ok(_) => tx.send(buffer.as_bytes().iter().copied().collect()).unwrap(),
                Err(err) => { eprint!("{:?}", err); break; },
            }
        });
        rx
    }
}

impl Stream for CachedStreamReader {
    fn len(&self) -> usize {
        self.buffer.len()
    }

    // Wait on any data at all; Returns true if file is still open
    fn wait(&mut self) -> bool {
        if let Some(rx) = &self.rx {
            match rx.recv() {
                Ok(mut data) => self.buffer.append(&mut data),
                Err(_) => self.rx = None,
            }
        }

        !self.rx.is_none()
    }
}

use std::io::Read;
impl  Read for CachedStreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // FIXME: Call fill_buffer() only if pos is "close" to the end of the buffer
        let start = self.pos as usize;
        self.fill_buffer(start + buf.len());
        let len = buf.len().min(self.len().saturating_sub(start));
        if len > 0 {
            let end = start + len;
            buf[..len].copy_from_slice(&self.buffer[start..end]);
            self.pos = self.pos.saturating_add(len as u64);
        }
        Ok(len)
    }
}

use std::io::{Seek, SeekFrom};
impl  Seek for CachedStreamReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let (start, offset) = match pos {
            SeekFrom::Start(n) => (0_i64, n as i64),
            SeekFrom::Current(n) => (self.pos as i64, n),
            SeekFrom::End(n) => (self.len() as i64, n),
        };
        self.pos = (((start as i64).saturating_add(offset)) as u64).min(self.len() as u64);
        Ok(self.pos)
    }
}

// BufReader<CachedStreamReader> is unnecessary and results in extra copies. Avoid using it, and just use our impl instead.
impl  std::io::BufRead for CachedStreamReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.fill_buffer(self.pos as usize);
        Ok(&self.buffer[self.pos as usize..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt as u64;
    }
}