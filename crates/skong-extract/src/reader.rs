//! A bounds-checked little/big-endian cursor over a byte slice, with Unity's 4-byte
//! alignment and length-prefixed strings.
use anyhow::{Result, bail};

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big_endian: bool) -> Self {
        Self { buf, pos: 0, big_endian }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_big_endian(&mut self, big: bool) {
        self.big_endian = big;
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            bail!("unexpected end of data (want {n} bytes at {}, have {})", self.pos, self.remaining());
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    /// Skip to the next multiple of 4 (Unity's `AlignBytes`).
    pub fn align(&mut self) -> Result<()> {
        let pad = (4 - self.pos % 4) % 4;
        self.skip(pad.min(self.remaining()))
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a: [u8; N] = self.bytes(N)?.try_into().expect("length checked");
        if self.big_endian {
            a.reverse();
        }
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.arr()?))
    }

    /// A non-negative array/string length, sanity-checked against what is left.
    pub fn len(&mut self, elem_size: usize) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 || (n as usize).saturating_mul(elem_size.max(1)) > self.remaining() {
            bail!("bad length {n} at {}", self.pos - 4);
        }
        Ok(n as usize)
    }

    /// NUL-terminated string.
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.buf[self.pos..];
        let Some(end) = rest.iter().position(|&b| b == 0) else { bail!("unterminated string at {}", self.pos) };
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }

    /// Unity `string`: i32 length, UTF-8 bytes, aligned to 4.
    pub fn string(&mut self) -> Result<String> {
        let n = self.len(1)?;
        let s = String::from_utf8_lossy(self.bytes(n)?).into_owned();
        self.align()?;
        Ok(s)
    }

    /// Unity `vector<UInt8>`: i32 length, bytes, aligned to 4.
    pub fn byte_array(&mut self) -> Result<&'a [u8]> {
        let n = self.len(1)?;
        let b = self.bytes(n)?;
        self.align()?;
        Ok(b)
    }

    /// `PPtr<T>`: file index (0 = this file) and path id.
    pub fn pptr(&mut self) -> Result<PPtr> {
        Ok(PPtr { file: self.i32()?, path_id: self.i64()? })
    }
}

/// A reference to an object: `file` 0 is the same serialized file, `n` the n-th external.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PPtr {
    pub file: i32,
    pub path_id: i64,
}

impl PPtr {
    pub fn is_null(&self) -> bool {
        self.path_id == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_and_big_endian_strings_and_alignment() {
        let mut buf = vec![1u8, 0, 0, 0];
        buf.extend(3i32.to_le_bytes());
        buf.extend(b"abc\0");
        buf.extend([0xAA]);
        let mut r = Reader::new(&buf, false);
        assert_eq!(r.u32().unwrap(), 1);
        assert_eq!(r.string().unwrap(), "abc");
        assert_eq!(r.pos(), 12);
        assert_eq!(r.u8().unwrap(), 0xAA);
        assert!(r.u8().is_err(), "reading past the end is an error, not a panic");

        let mut r = Reader::new(&[0, 0, 0, 22, b'x', 0], true);
        assert_eq!(r.u32().unwrap(), 22);
        assert_eq!(r.cstr().unwrap(), "x");
    }

    #[test]
    fn rejects_absurd_lengths() {
        let buf = 1000i32.to_le_bytes();
        assert!(Reader::new(&buf, false).string().is_err());
        let buf = (-1i32).to_le_bytes();
        assert!(Reader::new(&buf, false).byte_array().is_err());
    }
}
