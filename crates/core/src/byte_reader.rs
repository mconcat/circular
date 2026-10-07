
use std::marker::PhantomData;

pub trait Endian {
    fn u16(bytes: [u8; 2]) -> u16;
    fn u32(bytes: [u8; 4]) -> u32;
    fn u64(bytes: [u8; 8]) -> u64;
    fn i32(bytes: [u8; 4]) -> i32;
    fn i64(bytes: [u8; 8]) -> i64;
}

#[derive(Clone, Copy, Debug)]
pub enum BigEndian {}

#[derive(Clone, Copy, Debug)]
pub enum LittleEndian {}

impl Endian for BigEndian {
    fn u16(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }
    fn u32(bytes: [u8; 4]) -> u32 {
        u32::from_be_bytes(bytes)
    }
    fn u64(bytes: [u8; 8]) -> u64 {
        u64::from_be_bytes(bytes)
    }
    fn i32(bytes: [u8; 4]) -> i32 {
        i32::from_be_bytes(bytes)
    }
    fn i64(bytes: [u8; 8]) -> i64 {
        i64::from_be_bytes(bytes)
    }
}

impl Endian for LittleEndian {
    fn u16(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }
    fn u32(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }
    fn u64(bytes: [u8; 8]) -> u64 {
        u64::from_le_bytes(bytes)
    }
    fn i32(bytes: [u8; 4]) -> i32 {
        i32::from_le_bytes(bytes)
    }
    fn i64(bytes: [u8; 8]) -> i64 {
        i64::from_le_bytes(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteReadError {
    Truncated,
    LengthOverflow,
}

#[derive(Clone, Debug)]
pub struct ByteReader<'b, E: Endian> {
    bytes: &'b [u8],
    at: usize,
    endian: PhantomData<E>,
}

impl<'b, E: Endian> ByteReader<'b, E> {
    #[must_use]
    pub const fn new(bytes: &'b [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            endian: PhantomData,
        }
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.at
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    #[must_use]
    pub const fn finished(&self) -> bool {
        self.at == self.bytes.len()
    }

    #[must_use]
    pub fn rest(&self) -> &'b [u8] {
        &self.bytes[self.at..]
    }

    pub fn take(&mut self, count: usize) -> Result<&'b [u8], ByteReadError> {
        let end = self
            .at
            .checked_add(count)
            .ok_or(ByteReadError::LengthOverflow)?;
        let bytes = self
            .bytes
            .get(self.at..end)
            .ok_or(ByteReadError::Truncated)?;
        self.at = end;
        Ok(bytes)
    }

    pub fn byte(&mut self) -> Result<u8, ByteReadError> {
        Ok(self.take(1)?[0])
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ByteReadError> {
        Ok(self
            .take(N)?
            .try_into()
            .expect("take returned exactly N bytes"))
    }

    pub fn u16(&mut self) -> Result<u16, ByteReadError> {
        Ok(E::u16(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, ByteReadError> {
        Ok(E::u32(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, ByteReadError> {
        Ok(E::u64(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, ByteReadError> {
        Ok(E::i32(self.array()?))
    }

    pub fn i64(&mut self) -> Result<i64, ByteReadError> {
        Ok(E::i64(self.array()?))
    }

    pub fn len_prefixed(&mut self) -> Result<&'b [u8], ByteReadError> {
        let length = self.u32()? as usize;
        self.take(length)
    }

    pub fn flag(&mut self) -> Result<Result<bool, u8>, ByteReadError> {
        Ok(match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_follow_the_type_argument_endianness() {
        let bytes = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(ByteReader::<BigEndian>::new(&bytes).u32(), Ok(0x0102_0304));
        assert_eq!(
            ByteReader::<LittleEndian>::new(&bytes).u32(),
            Ok(0x0403_0201)
        );
        assert_eq!(
            ByteReader::<BigEndian>::new(&bytes).u64(),
            Ok(0x0102_0304_0506_0708)
        );
        assert_eq!(
            ByteReader::<LittleEndian>::new(&bytes).u64(),
            Ok(0x0807_0605_0403_0201)
        );
        assert_eq!(ByteReader::<BigEndian>::new(&bytes).u16(), Ok(0x0102));
        assert_eq!(
            ByteReader::<LittleEndian>::new(&[0xff, 0xff, 0xff, 0xff]).i32(),
            Ok(-1)
        );
        assert_eq!(
            ByteReader::<BigEndian>::new(&[0xff, 0, 0, 0, 0, 0, 0, 0]).i64(),
            Ok(-(1_i64 << 56))
        );
    }

    #[test]
    fn the_cursor_advances_and_reports_what_is_left() {
        let bytes = [0, 0, 0, 2, 0xaa, 0xbb, 0xcc];
        let mut reader = ByteReader::<BigEndian>::new(&bytes);
        assert_eq!(reader.len_prefixed(), Ok(&[0xaa, 0xbb][..]));
        assert_eq!(reader.position(), 6);
        assert_eq!(reader.remaining(), 1);
        assert_eq!(reader.rest(), &[0xcc]);
        assert!(!reader.finished());
        assert_eq!(reader.byte(), Ok(0xcc));
        assert!(reader.finished());
        assert_eq!(reader.byte(), Err(ByteReadError::Truncated));
    }

    #[test]
    fn short_input_and_overflowing_lengths_are_distinct_refusals() {
        let mut reader = ByteReader::<LittleEndian>::new(&[1, 2, 3]);
        assert_eq!(reader.u32(), Err(ByteReadError::Truncated));
        assert_eq!(
            reader.position(),
            0,
            "a refused read does not move the cursor"
        );
        reader.byte().expect("one byte is there");
        assert_eq!(reader.take(usize::MAX), Err(ByteReadError::LengthOverflow));
        let mut prefixed = ByteReader::<BigEndian>::new(&[0, 0, 0, 5, 1]);
        assert_eq!(prefixed.len_prefixed(), Err(ByteReadError::Truncated));
    }

    #[test]
    fn a_flag_is_exactly_zero_or_one() {
        let mut reader = ByteReader::<BigEndian>::new(&[0, 1, 2]);
        assert_eq!(reader.flag(), Ok(Ok(false)));
        assert_eq!(reader.flag(), Ok(Ok(true)));
        assert_eq!(reader.flag(), Ok(Err(2)));
        assert_eq!(reader.flag(), Err(ByteReadError::Truncated));
    }
}
