// Reader of text files

use std::path::PathBuf;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::fmt;

use crate::files::LogFileTrait;

pub struct TextLogFile {
    // pub file_path: PathBuf,
    file: File,
}

impl fmt::Debug for TextLogFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextLogFile")
         .field("bytes", &self.len())
         .finish()
    }
}

impl LogFileTrait for TextLogFile {
    fn len(&self) -> usize {
        self.file.metadata().unwrap().len() as usize
    }

    fn quench(&mut self) {}

    fn read(&mut self, offset: usize, len: usize) -> Option<Vec<u8>> {
        if offset > self.len() {
            None
        } else {
            let end = (offset + len).min(self.len());
            let mut buf = vec![0u8; end-offset];
            match self.file.seek(SeekFrom::Start(offset as u64)) {
                Err(_) => None,
                Ok(_pos) => {
                    match self.file.read(&mut buf) {
                        Err(_) => None,  // TODO: Log an error somewhere?
                        Ok(actual) => {
                            assert!(actual <= len);
                            buf.truncate(actual);
                            Some(buf)
                        },
                    }
                }
            }
        }
    }

    fn chunk(&self, target: usize) -> (usize, usize) {
        let chunk_size = 1024 * 1024;
        let start = target.saturating_sub(chunk_size / 2);
        let end = (start + chunk_size).min(self.len());
        let start = end.saturating_sub(chunk_size);
        (start, end)
    }
}

impl TextLogFile {
    pub fn new(filename: PathBuf) -> std::io::Result<TextLogFile> {
        Ok(TextLogFile {
            // file_path: input_file.unwrap(),
            file: File::open(filename)?,
        })
    }
}
