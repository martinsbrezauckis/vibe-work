//! Streaming JSON and physical-line JSONL validation. Uses `serde_json`; called before atomic rename.
//! `IgnoredAny` skips token contents without building a document tree or buffering entire lines.

use std::io::{self, BufRead, BufReader, Read};

use serde::{Deserialize, de::IgnoredAny};

use crate::FileSkipReason;

pub(crate) fn validate(reader: impl Read, jsonl: bool) -> Result<(), FileSkipReason> {
    let mut reader = BufReader::new(reader);
    if jsonl {
        while !reader
            .fill_buf()
            .map_err(|_| FileSkipReason::Io)?
            .is_empty()
        {
            let mut line = Line {
                reader: &mut reader,
                ended: false,
                remaining: 0,
                ends_here: false,
            };
            json(&mut line)?;
        }
        Ok(())
    } else {
        json(reader)
    }
}

fn json(reader: impl Read) -> Result<(), FileSkipReason> {
    let mut parser = serde_json::Deserializer::from_reader(reader);
    IgnoredAny::deserialize(&mut parser)
        .and_then(|_| parser.end())
        .map_err(|error| {
            if error.is_io() {
                FileSkipReason::Io
            } else {
                FileSkipReason::InvalidJson
            }
        })
}

// Bound the standard JSON parser's reader at each physical newline, without allocating a line.
struct Line<'a, R> {
    reader: &'a mut R,
    ended: bool,
    remaining: usize,
    ends_here: bool,
}

impl<R: BufRead> Read for Line<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.ended || output.is_empty() {
            return Ok(0);
        }
        let available = self.reader.fill_buf()?;
        if self.remaining == 0 {
            let newline = memchr::memchr(b'\n', available);
            self.remaining = newline.map_or(available.len(), |at| at + 1);
            self.ends_here = newline.is_some();
        }
        let length = output.len().min(self.remaining);
        output[..length].copy_from_slice(&available[..length]);
        self.remaining -= length;
        self.ended = self.ends_here && self.remaining == 0;
        self.reader.consume(length);
        Ok(length)
    }
}
