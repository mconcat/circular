use super::{Error, invalid};

pub fn uint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
pub fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    uint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}
pub fn fixed_field(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    out.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| invalid())?
            .to_be_bytes(),
    );
    out.extend_from_slice(bytes);
    Ok(())
}
#[derive(Clone)]
pub struct Reader<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or_else(invalid)?;
        let bytes = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(bytes)
    }
    pub fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn uint(&mut self) -> Result<u64, Error> {
        let mut n = 0;
        for i in 0..10 {
            let b = self.byte()?;
            if i == 9 && b > 1 {
                return Err(invalid());
            }
            n |= u64::from(b & 127) << (i * 7);
            if b < 128 {
                if i > 0 && b == 0 {
                    return Err(invalid());
                }
                return Ok(n);
            }
        }
        Err(invalid())
    }
    pub fn field(&mut self) -> Result<&'a [u8], Error> {
        let n = usize::try_from(self.uint()?).map_err(|_| invalid())?;
        self.take(n)
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn fixed_field(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u32()?;
        self.take(n as usize)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let bytes = &self.bytes[self.at..];
        self.at = self.bytes.len();
        bytes
    }
    pub fn done(&self) -> Result<(), Error> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}
