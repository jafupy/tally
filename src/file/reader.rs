use crate::counter::BUFFER_BYTES;
use std::{
    fs::File,
    io::{self, BufRead, Read},
};

pub fn buffer() -> Vec<u8> {
    vec![0; BUFFER_BYTES]
}

pub struct Reusable<'a> {
    file: File,
    buffer: &'a mut [u8],
    position: usize,
    filled: usize,
}

impl<'a> Reusable<'a> {
    pub fn open(file: File, buffer: &'a mut [u8]) -> Self {
        debug_assert!(buffer.len() >= BUFFER_BYTES);
        Self {
            file,
            buffer: &mut buffer[..BUFFER_BYTES],
            position: 0,
            filled: 0,
        }
    }
}

impl Read for Reusable<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let amount = available.len().min(output.len());
        output[..amount].copy_from_slice(&available[..amount]);
        self.consume(amount);
        Ok(amount)
    }
}

impl BufRead for Reusable<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.filled {
            self.filled = self.file.read(self.buffer)?;
            self.position = 0;
        }
        Ok(&self.buffer[self.position..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.filled);
    }
}
