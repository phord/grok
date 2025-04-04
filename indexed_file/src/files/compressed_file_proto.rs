// Compressed file reader trait
use std::io::BufRead;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

use crate::files::Stream;

/// Breadcrumb holds information about subsections of the compressed data we can seek to.
///
/// Some formats support these breadcrumbs natively (like gzip streams or zstd frames).  But even when they
/// are supported, they're not always available. These kinds of crumbs are ideal because they let us decompress
/// sections of the file independently from others, making random read access very fast.
///
/// If the native breadcrumbs are missing or unsupported, we may be able to save some decompressor context
/// information the first time we decompress the file, and then use this context later to decompress just a
/// subsection again.
///
/// If context resume is not supported, we may have to decompress the whole file again to get to the right
/// spot. This is slow, but it's better than not being able to seek at all.  Another option is to keep the
/// decompressed data in memory or spooling it to a file rather than decompressing it again. The read_buffer
/// struct can help by preserving part of a decompressed file for this kind of navigation.
///
/// scan_frames() will attempt to index the whole file by recording every native breadcrumb in the
/// file. If a crumb's uncompressed size is unknown, we stop scanning and leave the rest of the index
/// "unknown". As we decode data through normal reads, we will learn the length of each frame and we can
/// fill in the missing information (len). We will then push a new unknown crumb size into the index
/// representing the new unknown frontier of the logical space in frames.

struct Breadcrumb<context> {
    // The physical offset of the start of the crumb in the compressed file
    physical: u64,

    // The logical offset of the decoded data in the decompressed file for this crumb
    logical: u64,

    // The length of the decompressed data in this crumb in bytes, if known. Zero means frontier.
    len: u64,
}

mod read_buffer;
use read_buffer::ReadBuffer;

trait CompressedFileReader {
    fn new(file: R) -> std::io::Result<Self>;
    fn is_recognized(file: R) -> bool;
    fn get_length(&self) -> usize;
    fn wait(&mut self) -> bool;
}

pub struct CompressedFile<R, Decomp> {
    /// The source (compressed) file reader
    file: R,

    /// The size of the compressed file in bytes
    source_bytes: u64,

    /// The format decompressor context
    decoder: Decomp,

    /// Sorted logical -> physical file offsets
    frames: Vec<Breadcrumb>,

    /// The last frame we seeked to
    cur_crumb: usize,

    /// Logical position in file (decompressed bytes position)
    pos: u64,

    /// Logical position in decode stream
    seek_pos: Option<u64>,

    /// Buffer for BufRead
    read_buffer: ReadBuffer,
}

impl<R> CompressedFile<R> {
    /// Find the indexed frame that holds or is closest to a given uncompressed offset
    fn lookup_frame_index(&self, pos: u64) -> usize {
        // Avoid binary-search lookup if target frame is near the current_frame (common)
        const NEARBY:usize = 2;
        let start = if pos < self.pos { self.cur_crumb.saturating_sub(NEARBY) } else { self.cur_crumb };
        let end =  if pos > self.pos { (start + NEARBY).min(self.frames.len()) } else { self.cur_crumb };
        let mut index = end;
        for ind in start..end {
            let frame = &self.frames[ind];
            let frame_range = frame.logical..frame.logical+frame.len;
            if frame_range.contains(&pos) {
                index = ind;
                break;
            }
        }

        if index == end {
            // Did not find it. Search the slow way.

            index = match self.frames.binary_search_by_key(&pos, |f| f.logical) {
                Err(n) => n - 1,
                Ok(n) => n,
            };
        }
        index
    }
}

impl<R: Read + Seek> CompressedFile<R, Decomp> {
    pub fn new(mut file: R) -> std::io::Result<Self> {
        // TODO: Return error if no file or not known type
        let source_bytes = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;
        let decoder = Decomp::default();

        let mut cf = Self {
            file,
            source_bytes,
            decoder,
            pos: 0,
            seek_pos: None,
            frames: Vec::new(),
            cur_crumb: 0,
            read_buffer: ReadBuffer::new(),
        };

        // Read all physical frame sizes into self.frames.
        cf.scan_frames().expect("File format is valid");

        cf.file.seek(SeekFrom::Start(0))?;

        Ok(cf)
    }


    /// TODO continue from here


    pub fn is_recognized(mut file: R) -> bool {
        if file.seek(SeekFrom::Start(0)).is_err() {
            false
        } else {
            match read_frame_header(&mut file) {
                Ok((frame, _bytes_read)) => {
                    frame.check_valid().is_ok()
                },
                _ => false,
            }
        }
    }

    // Scan all the zstd frame headers in the file and record their positions and sizes, if known
    fn scan_frames(&mut self) -> Result<(), ReadFrameHeaderError> {
        let mut pos = 0;

        let mut fpos = 0;
        while fpos < self.source_bytes {
            // Starting a new frame.  Record details.
            let (uncompressed_bytes, frame_bytes) = self.skip_frame()?;
            match uncompressed_bytes {
                None => {
                    // No point continuing the scan because we don't know the uncompressed size
                    // Leave an empty marker for the last physical frame position
                    let frame = Breadcrumb { physical: fpos, logical: pos, len: 0};
                    self.frames.push(frame);
                    break
                },
                Some(0) => { /* Skippable; no action */ },
                Some(size) => {
                    let frame = Breadcrumb { physical: fpos, logical: pos, len: size};
                    // eprintln!("Frame @ {fpos} holds {pos} to {}", pos+size);
                    self.frames.push(frame);
                    pos += size;
                }
            }
            fpos += frame_bytes;
            assert_eq!(fpos, self.file.stream_position().unwrap() as u64);
        }
        Ok(())
    }

    fn skip_frame(&mut self) -> Result<(Option<u64>, u64), ReadFrameHeaderError> {
        match read_frame_header(&mut self.file) {
            Err(ReadFrameHeaderError::SkipFrame(_magic_num, skip_size,)) => {
                self.file.seek(SeekFrom::Current(skip_size as i64)).unwrap();
                // Skipped a frame with no uncompressible bytes
                // FIXME: Magic number "4" is the size of the frame header we parsed. read_frame_header should tell us that.
                Ok((Some(0), 4u64 + skip_size as u64))
            }
            Ok((frame, bytes_read)) => {
                // Started a new frame. Skip all the blocks.
                let mut bytes_read = bytes_read as u64;
                let mut block_dec = block_decoder::new();
                loop {
                    let (block_header, block_header_size) = block_dec
                        .read_block_header(&mut self.file)
                        .map_err(FrameDecoderError::FailedToReadBlockHeader).expect("TODO: Map error to some common err");

                    // block_header.decompressed_size is usually filled only after decoding the block  :-(
                    bytes_read += block_header_size as u64;
                    self.file.seek(SeekFrom::Current(block_header.content_size as i64)).unwrap();
                    bytes_read += block_header.content_size as u64;
                    if block_header.last_block {
                        break;
                    }
                }
                if frame.header.descriptor.content_checksum_flag() {
                    self.file.seek(SeekFrom::Current(4)).unwrap();
                    bytes_read += 4;
                }
                // Return the uncompressed size or None if we don't know
                let uncompressed_bytes = match frame.header.frame_content_size() {
                    Ok(size) => Some(size),
                    Err(_) => None,
                };
                Ok((uncompressed_bytes, bytes_read))
            },
            Err(other) => {
                // Some error.  Quit early.
                return Err(other)
            },
        }
    }

    // Position to the start of a different frame because of an explicit seek()
    fn goto_frame(&mut self, index: usize) {
        let frame = &self.frames[index];

        // Position file to start of frame
        if self.file.stream_position().unwrap() != frame.physical {
            self.file.seek(SeekFrom::Start(frame.physical)).expect("Seek does not fail");
        }
        // reset read_buffer
        if frame.logical != self.read_buffer.end() {
            self.read_buffer = ReadBuffer::new();
        }

        self.pos = frame.logical;
        self.begin_frame();
        self.cur_crumb = index;
    }

    fn has_file_size(&self) -> bool {
        self.frames.last().unwrap().len != 0
    }
    // Update last frame if we just decoded the last byte
    fn end_frame(&mut self) {
        // We may not be on the last frame in the index, but we only update the last frame.  We assume that we are not
        // decoding some earlier frame because we cannot know the logical offset of any frame after the unknown frontier one.
        let frame = self.frames.last_mut().unwrap();
        if frame.len == 0 {
            let logical_pos = self.pos + self.decoder.can_collect() as u64;
            if logical_pos > frame.logical {
                frame.len = logical_pos - frame.logical;

                // Push a new last-unknown-frame if we're not at EOF yet
                let fpos = self.file.stream_position().unwrap() as u64;
                assert!(fpos > frame.physical);

                if fpos < self.source_bytes {
                    self.frames.push(Breadcrumb { physical: fpos, logical: logical_pos, len: 0 } );
                }
            }
        }
    }

    // Parse a frame header and automatically skip over Skippable Frames
    fn begin_frame(&mut self) {
        while self.file.stream_position().unwrap() < self.source_bytes {
            match self.decoder.reset(&mut self.file) {
                Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame(
                    _magic_num,
                    skip_size,
                ))) => {
                    self.file.seek(SeekFrom::Current(skip_size as i64)).unwrap();
                    // TODO: If last self.frame points to us, we should move it to point to the next frame instead.
                    continue;
                }
                Ok(_) => {
                    break
                },
                other => {
                    // FIXME: Report this error upstream
                    other.unwrap(); // Report the error and panic
                    break
                },
            }
        }
    }

    fn apply_seek(&mut self) -> Result<(), std::io::Error> {
        if let Some(pos) = self.seek_pos {
            // Forget this for next time
            self.seek_pos = None;

            if pos == self.pos {
                // no-op
                return Ok(())
            }

            // Move to a new position
            if self.read_buffer.seek_to(pos) {
                // Found pos in read_buffer.  All done.
                self.pos = pos;
                // TODO: Adjust cur_crumb to match?
            } else {
                let index = self.lookup_frame_index(pos);

                let frame = &self.frames[index];
                let frame_range = frame.logical..frame.logical+frame.len;
                assert!(frame_range.contains(&pos) || frame.len == 0);
                if pos < self.pos || !frame_range.contains(&self.pos) {
                    // Open a new frame
                    self.goto_frame(index);
                }

                if pos > self.pos {
                    // We're in the right frame, but we're behind
                    self.skip_bytes(pos - self.pos)?;
                    assert_eq!(pos, self.pos, "seek pos is outside of file range");
                }
            }
        }
        Ok(())
    }

    // Ok(true) at eof
    fn decode_more_bytes(&mut self) -> Result<bool, std::io::Error> {
        loop {
            if self.decoder.can_collect() > 0 {
                // You've already got bytes.  Go away.
                return Ok(false)
            } else if self.decoder.is_finished() {
                if self.file.stream_position().unwrap() >= self.source_bytes {
                    // EOF
                    return Ok(true)
                }
                // Start a new frame
                self.begin_frame();
            } else {
                // Decode more bytes
                match self.decoder.decode_blocks(&mut self.file, BlockDecodingStrategy::UptoBlocks(1)) {
                    Ok(_) => {
                        if self.decoder.is_finished() {
                            // Reached end of frame
                            self.end_frame();
                        }
                    }
                    Err(e) => {
                        let err = std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Error in the zstd decoder: {:?}", e),
                        );
                        return Err(err);
                    }
                }
            }
        }
    }

    fn skip_bytes(&mut self, count:u64) -> Result<(), std::io::Error> {
        let target = self.pos + count;
        while self.pos < target {
            self.decode_into_buffer()?;
            let avail = self.read_buffer.remaining().min(target - self.pos);
            if avail > 0 {
                self.pos += avail;
                self.read_buffer.consume(avail);
            }
        }
        Ok(())
    }

    // Decode more bytes from the compressed source into our buffer if needed
    fn decode_into_buffer(&mut self) -> Result<(), std::io::Error> {
        const BUFFER_THRESHOLD_EDGE:u64 = 40 * 1024;
        const BUFFER_THRESHOLD_CAPACITY:u64 = 10 * 1024 * 1024;
        if self.read_buffer.remaining() < BUFFER_THRESHOLD_EDGE {
            self.decode_more_bytes()?;
            if self.decoder.can_collect() > 0 {
                if let Some(buffer) = self.decoder.collect() {
                    // Add more bytes to our internal buffer
                    self.read_buffer.extend(buffer, self.pos);

                    // TODO: Add a test to ensure this bounding works as expected
                    // Discard start of buffer if we're well past it now
                    let cap = BUFFER_THRESHOLD_CAPACITY;
                    // TODO: Push this down into ReadBuffer::extend()
                    if self.read_buffer.len() > cap as usize * 3
                            && self.read_buffer.consumed >= cap * 2 {
                        self.read_buffer.discard_front(cap);
                    }
                }
            }
        }
        Ok(())
    }

    // Move the stream position to read more bytes at current logical pos
    fn update_stream(&mut self) -> Result<(), std::io::Error> {
        self.apply_seek()?;
        self.decode_into_buffer()
    }
}

impl<R: Read + Seek> Seek for CompressedFile<R> {
    fn seek(&mut self, target: SeekFrom) -> Result<u64, std::io::Error> {
        let (start, offset) = match target {
            SeekFrom::Start(n) => (0, n as i64),
            SeekFrom::Current(n) => (self.pos, n),
            SeekFrom::End(n) =>
                if self.has_file_size() {
                    (self.source_bytes, n)
                } else {
                    // Ideally we could SeekFrom::End(-1000) and only decode the last frame even if we don't know
                    // all the frames' decompressed sizes yet. But we wouldn't be able to return the current offset
                    // from Start in that case, which the API requires.
                    todo!("We don't know if we know the end-of-file pos yet");
                },
        };
        let pos = start.saturating_add_signed(offset).min(self.get_length() as u64);

        // Save the seek position for the future
        self.seek_pos = Some(pos);
        Ok(pos)
    }
}

impl<R: Read + Seek> Read for CompressedFile<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut bytes = 0;
        while bytes < buf.len() {
            self.update_stream()?;

            let actual = (self.read_buffer.remaining() as usize).min(buf.len() - bytes);
            buf[bytes..bytes+actual].copy_from_slice(&self.read_buffer.get_buffer()[..actual]);

            self.pos += actual as u64;
            self.read_buffer.consume(actual as u64);
            bytes += actual;
            if actual == 0 {  // EOF
                break;
            }
        }
        Ok(bytes)
    }
}

impl<R: Read + Seek> BufRead for CompressedFile<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        // FIXME: We have to copy bytes twice here: Once from the Decoder buffer to ours, and once again
        // to our reader.  We could skip the first copy if we had access to Decoder::buffer::as_slices(), but
        // Decoder::buffer is private.  Shucks.  For now, we must copy.
        self.update_stream()?;
        Ok(self.read_buffer.get_buffer())
    }

    fn consume(&mut self, amt: usize) {
        assert!((amt as u64) <= self.read_buffer.remaining());
        self.pos += amt as u64;
        self.read_buffer.consume(amt as u64);
    }
}

impl<R> Stream for CompressedFile<R> {
    fn get_length(&self) -> usize {
        let last = &self.frames.last().unwrap();
        let len = last.logical + last.len +
            if last.len > 0 { 0 } else {
                // estimate some extra bytes based on remaining compressed data
                assert!(self.source_bytes >= last.physical);
                self.source_bytes - last.physical
            };
        len as usize
    }
    // Poll for new data
    fn wait(&mut self) -> bool {
        true
    }
}

#[test]
fn test_compressed_file() {
    use std::fs::File;
    // HACKY FILENAME
    let path = "/home/phord/git/mine/igrok/test.zst".to_owned();
    let file = File::open(path).expect("File exists");

    let mut comp = CompressedFile::new(&file).unwrap();
    match std::io::copy(&mut comp, &mut std::io::stdout().lock()) {
        Err(e) => eprintln!("Error: {:?}", e),
        Ok(_) => (),
    }
}


#[test]
fn test_compressed_file_seek() {
    use std::io::{BufRead, BufReader};
    use std::fs::File;
    // HACKY FILENAME
    let path = "/home/phord/git/mine/igrok/test.zst".to_owned();
    let file = File::open(path).expect("File exists");

    let comp = CompressedFile::new(&file).unwrap();
    let mut reader = BufReader::new(comp);
    let mut line6 = String::default();
    let mut first_5_lines = String::default();
    for _ in 0..5 {
        reader.read_line(&mut first_5_lines).expect("Can read 5 lines");
    }
    let count = first_5_lines.len() as u64;
    reader.read_line(&mut line6).expect("Can read 6 lines");

    assert!(!line6.is_empty());

    let mut comp = CompressedFile::new(&file).unwrap();
    comp.seek(SeekFrom::Start(count)).expect("Seek should work");
    let mut reader = BufReader::new(comp);
    let mut line6b = String::default();
    reader.read_line(&mut line6b).expect("Can read 6 lines");

    assert_eq!(line6, line6b);

}
