use super::{BUFFER_BYTES, DETECTION_PREFIX_BYTES};
use std::fs::File;
use std::io::{self, BufRead, Read};

pub(super) struct ReusableBufReader<'a> {
    file: File,
    buffer: &'a mut [u8],
    position: usize,
    filled: usize,
}

impl<'a> ReusableBufReader<'a> {
    pub(super) fn new(file: File, buffer: &'a mut [u8]) -> Self {
        debug_assert!(buffer.len() >= BUFFER_BYTES);
        Self {
            file,
            buffer: &mut buffer[..BUFFER_BYTES],
            position: 0,
            filled: 0,
        }
    }
}

impl Read for ReusableBufReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let amount = available.len().min(output.len());
        output[..amount].copy_from_slice(&available[..amount]);
        self.consume(amount);
        Ok(amount)
    }
}

impl BufRead for ReusableBufReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.filled {
            let _read_span = trace_span!("file_read", None);
            self.filled = self.file.read(self.buffer)?;
            trace_event!(read_chunk, None, self.filled);
            self.position = 0;
        }
        Ok(&self.buffer[self.position..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.filled);
    }
}

pub(super) fn read_prefix(reader: &mut impl BufRead) -> io::Result<&[u8]> {
    let buffer = reader.fill_buf()?;
    Ok(&buffer[..buffer.len().min(DETECTION_PREFIX_BYTES)])
}
