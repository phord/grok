/// A wrapper for a LogFileLines that applies color, filtering, caching, etc.

use crossterm::style::Color;
use crate::{config::Config, styled_text::{styled_line::{PattColor, StyledLine}, stylist::Stylist, LineViewMode}};
use fnv::FnvHasher;
use std::hash::Hasher;
use lazy_static::lazy_static;
use regex::Regex;
use indexed_file::{files::Stream, indexer::indexed_log::IndexStats, IndexedLog, Log, LogLine, LogStack};
pub struct Document {
    // FIXME: StyledLine caching -- premature optimization?
    // File contents
    log: LogStack,
    stylist: Stylist,
}

impl Document {

    pub fn get_lines_range<'a, R>(&'a mut self, range: &'a R) -> impl DoubleEndedIterator<Item = LogLine> + 'a
    where R: std::ops::RangeBounds<usize> {
        self.stylist.iter_range(&mut self.log, range)
    }

    pub fn get_plain_lines<'a, R>(&'a mut self, range: &'a R) -> impl DoubleEndedIterator<Item = LogLine> + 'a
    where R: std::ops::RangeBounds<usize> {
        self.log.iter_lines_range(range)
    }

    pub fn set_search(&mut self, search: &str) -> Result<(), regex::Error> {
        // TODO: Remove old search patter from stylist
        self.stylist.add_match(Regex::new(search)?, PattColor::Inverse);
        // TODO: force viewer to refresh page
        self.log.search_regex(search)
    }

    pub fn set_filter(&mut self, filter: &str) -> Result<(), regex::Error> {
        self.log.filter_regex(filter)
    }

    pub fn search_next(&mut self, line: usize, repeat: usize) -> Option<usize> {
        self.log.search_next(repeat, line)
    }

    pub fn search_back(&mut self, line: usize, repeat: usize) -> Option<usize> {
        self.log.search_next_back(repeat, line)
    }

    pub fn run(&mut self, timeout: u64) -> Option<usize> {
        self.log.run_pending(timeout)
    }

    pub fn has_pending(&self) -> bool {
        self.log.has_pending()
    }

    pub fn poll(&mut self, timeout: Option<std::time::Instant>) -> usize {
        self.log.poll(timeout)
    }
}

impl Document {
    pub fn new(config: Config) -> Self {
        let filename = config.filename.first();
        let log = Log::open(filename).expect("Failed to open file");

        Self {
            log: LogStack::new(log),
            stylist: Stylist::default(),
        }
    }

    pub fn set_line_mode(&mut self, mode: LineViewMode) {
        self.stylist.mode = mode;
    }

    fn hash_color(&self, text: &str) -> Color {
        let mut hasher = FnvHasher::default();
        hasher.write(text.as_bytes());
        let hash = hasher.finish();

        let base = 0x80_u8;
        let red = (hash & 0xFF) as u8 | base;
        let green = ((hash >> 8) & 0xFF) as u8 | base;
        let blue = ((hash >> 16) & 0xFF) as u8 | base;

        Color::Rgb {r: red, g: green, b: blue}
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn info(&self) -> impl Iterator<Item = &IndexStats> + '_
    where Self: Sized
    {
        self.log.info()
    }

    // FIXME: Move to Stylist
    pub fn line_colors(&self, line: &str) -> StyledLine {
        // FIXME: Doesn't need &self

        lazy_static! {
            // TODO: Move these regexes to a config file
            // Apr  4 22:21:16.056 E8ABF4F03A6F I      vol.flush.cb ...
            static ref TIMESTAMP: Regex = Regex::new(r"(?x)
                ^(...\ [\ 1-3]\d\ [0-2]\d:[0-5]\d:\d{2}\.\d{3})\    # date & time
                 ([A-F0-9]{12})\                                    # PID
                 ([A-Z])\                                           # crumb").unwrap();

            static ref MODULE: Regex = Regex::new(r"(?x)
                 ^\ *([A-Za-z0-9_.]+)\                              # module
                 (?:\[([a-z0-9_.]+)\]){0,1}                         # submodule").unwrap();

            // Match any 0x{hex} number, any 16-digit all-uppercase hex number at word delimiters, or any decimal number which is not part of a word suffix.
            // TODO: Also match UUIDS and include units when attached, like `123GB`
            static ref NUMBER: Regex = Regex::new(r"[^A-Za-z.0-9_](\b0x[[:xdigit:]]+\b|\b[0-9A-F]{16}\b|(?:[[:digit:]]+\.)*[[:digit:]]+)").unwrap();
        }

        // FIXME: find a way to better separate sanitized line from styles, and merge the styles better
        // For now, I sanitize and then extract the line for the rest of this function
        let hold = StyledLine::sanitize_basic(line, PattColor::NoCrumb);
        let line = hold.line.as_str();

        let prefix = TIMESTAMP.captures(line);

        let mut styled = StyledLine::new(line, PattColor::NoCrumb);

        // Match and color PID and TIME
        let mut pos = 0;
        if let Some(p) = prefix {
            let crumb = p.get(3).unwrap().as_str();
            let default_style = match crumb {
                "E" => PattColor::Error,
                "A" => PattColor::Fail,
                _ => PattColor::Info,
            };

            styled.push(0, line.len(), default_style);

            let len = p.get(1).unwrap().end() + 1;
            styled.push(0, len, PattColor::Timestamp);

            // TODO: Calculate timestamp value?

            let pid = p.get(2).unwrap();
            let start = pid.start();
            let end = pid.end();
            let pid = pid.as_str();
            let pid_color = self.hash_color(pid);
            styled.push( start, end, PattColor::Pid(pid_color));

            // Match modules at start of line
            pos = end + 3;  // Skip over crumb; it will autocolor later
            let module = MODULE.captures(&line[pos..]);
            if let Some(m) = module {
                let first = m.get(1).unwrap();
                let color = self.hash_color(first.as_str());
                styled.push(pos + first.start(), pos + first.end(),PattColor::Module(color) );

                if let Some(second) = m.get(2) {
                    let color = self.hash_color(second.as_str());
                    styled.push(pos + second.start(), pos + second.end(), PattColor::Module(color));
                }
            }
        }

        for m in NUMBER.captures_iter(&line[pos..]) {
            let m = m.get(1).unwrap();
            let start = m.start();
            let end = m.end();
            let color = self.hash_color(m.as_str());
            styled.push( pos + start, pos + end , PattColor::Number(color) );
        }

        styled
    }
}