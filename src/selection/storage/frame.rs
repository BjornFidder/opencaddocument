use super::{SelectionStorageError, SelectionStorageErrorKind as Kind};
use std::io::{self, Write};

const MAGIC: &[u8; 8] = b"OCADSEL\0";
const HEADER: usize = 15;
pub(super) const BODY_LIMIT: usize = 64 * 1024 * 1024;
pub(super) const COUNT_LIMIT: usize = 262_144;
pub(super) const VALUE_LIMIT: usize = 1_048_576;
pub(super) const STRING_LIMIT: usize = 1_048_576;
const DEPTH_LIMIT: usize = 16;

type Result<T> = std::result::Result<T, SelectionStorageError>;

fn error(kind: Kind, offset: usize, message: &str) -> SelectionStorageError {
    SelectionStorageError {
        kind,
        offset: Some(offset),
        path: None,
        message: message.into(),
    }
}

pub(super) fn body(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() < HEADER {
        return Err(error(
            Kind::Truncated,
            bytes.len(),
            "incomplete selection header",
        ));
    }
    if &bytes[..8] != MAGIC {
        return Err(error(Kind::InvalidEnvelope, 0, "wrong selection magic"));
    }
    if bytes[8..10] != [0, 1] {
        return Err(error(
            Kind::UnsupportedVersion,
            8,
            "unsupported selection version",
        ));
    }
    if bytes[10] != 1 {
        return Err(error(
            Kind::UnsupportedKind,
            10,
            "unsupported selection kind",
        ));
    }
    let size = u32::from_be_bytes(bytes[11..15].try_into().unwrap()) as usize;
    if size > BODY_LIMIT {
        return Err(error(
            Kind::LimitExceeded,
            11,
            "selection body exceeds byte limit",
        ));
    }
    let end = HEADER + size;
    if bytes.len() < end {
        return Err(error(
            Kind::Truncated,
            bytes.len(),
            "incomplete selection body",
        ));
    }
    if bytes.len() > end {
        return Err(error(
            Kind::TrailingData,
            end,
            "bytes after selection frame",
        ));
    }
    let body = &bytes[HEADER..];
    preflight(body)?;
    Ok(body)
}

fn preflight(body: &[u8]) -> Result<()> {
    let mut scan = Scanner {
        input: body,
        pos: 0,
        remaining_values: VALUE_LIMIT,
    };
    scan.value(0, false)?;
    if scan.pos != body.len() {
        return Err(error(
            Kind::TrailingData,
            HEADER + scan.pos,
            "second value in selection body",
        ));
    }
    Ok(())
}

struct Scanner<'a> {
    input: &'a [u8],
    pos: usize,
    remaining_values: usize,
}

impl<'a> Scanner<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.input.len() - self.pos {
            return Err(error(
                Kind::Truncated,
                HEADER + self.input.len(),
                "incomplete MessagePack value",
            ));
        }
        let start = self.pos;
        self.pos += n;
        Ok(&self.input[start..self.pos])
    }

    fn length(&mut self, width: usize) -> Result<usize> {
        let mut n = 0_u32;
        for &byte in self.take(width)? {
            n = (n << 8) | u32::from(byte);
        }
        Ok(n as usize)
    }

    fn value(&mut self, depth: usize, key: bool) -> Result<()> {
        let offset = HEADER + self.pos;
        self.remaining_values = self
            .remaining_values
            .checked_sub(1)
            .ok_or_else(|| error(Kind::LimitExceeded, offset, "too many MessagePack values"))?;
        let marker = self.take(1)?[0];
        let string_len = match marker {
            0xa0..=0xbf => Some(usize::from(marker & 0x1f)),
            0xd9..=0xdb => Some(self.length(1 << (marker - 0xd9))?),
            _ => None,
        };
        if let Some(n) = string_len {
            if n > STRING_LIMIT {
                return Err(error(
                    Kind::LimitExceeded,
                    offset,
                    "string exceeds byte limit",
                ));
            }
            let text = self.take(n)?;
            return std::str::from_utf8(text)
                .map(|_| ())
                .map_err(|_| error(Kind::InvalidBody, offset, "invalid UTF-8"));
        }
        if key {
            return Err(error(Kind::InvalidBody, offset, "map key must be a string"));
        }
        let (len, map) = match marker {
            0x00..=0x7f | 0xc0 | 0xc2..=0xc3 | 0xe0..=0xff => return Ok(()),
            0xcc..=0xd3 => {
                self.take(1 << ((marker - 0xcc) & 3))?;
                return Ok(());
            }
            0x80..=0x8f => (usize::from(marker & 0x0f), true),
            0x90..=0x9f => (usize::from(marker & 0x0f), false),
            0xdc..=0xdd => (self.length(2 << (marker - 0xdc))?, false),
            0xde..=0xdf => (self.length(2 << (marker - 0xde))?, true),
            _ => {
                return Err(error(
                    Kind::InvalidBody,
                    offset,
                    "unsupported MessagePack marker",
                ))
            }
        };
        if depth >= DEPTH_LIMIT || len > COUNT_LIMIT {
            return Err(error(
                Kind::LimitExceeded,
                offset,
                "container exceeds count or depth limit",
            ));
        }
        let children = if map { len * 2 } else { len };
        if children > self.remaining_values {
            return Err(error(
                Kind::LimitExceeded,
                offset,
                "container exceeds remaining value budget",
            ));
        }
        if children > self.input.len() - self.pos {
            return Err(error(
                Kind::Truncated,
                HEADER + self.input.len(),
                "incomplete MessagePack container",
            ));
        }
        for i in 0..children {
            self.value(depth + 1, map && i % 2 == 0)?;
        }
        Ok(())
    }
}

pub(super) struct BoundedFrame {
    bytes: Vec<u8>,
    pub(super) exceeded: bool,
}

impl BoundedFrame {
    pub(super) fn new() -> Self {
        let mut bytes = Vec::from(*MAGIC);
        bytes.extend_from_slice(&[0, 1, 1, 0, 0, 0, 0]);
        Self {
            bytes,
            exceeded: false,
        }
    }

    pub(super) fn finish(mut self) -> Result<Vec<u8>> {
        let size = self.bytes.len() - HEADER;
        self.bytes[11..15].copy_from_slice(&(size as u32).to_be_bytes());
        preflight(&self.bytes[HEADER..])?;
        Ok(self.bytes)
    }
}

impl Write for BoundedFrame {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > BODY_LIMIT + HEADER - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("selection body exceeds byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
