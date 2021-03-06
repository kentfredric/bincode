use error::Result;
use serde;
use std::{io, slice};

/// An optional Read trait for advanced Bincode usage.
///
/// It is highly recommended to use bincode with `io::Read` or `&[u8]` before
/// implementing a custom `BincodeRead`.
///
/// The forward_read_* methods are necessary because some byte sources want
/// to pass a long-lived borrow to the visitor and others want to pass a
/// transient slice.
pub trait BincodeRead<'storage>: io::Read {
    /// Check that the next `length` bytes are a valid string and pass
    /// it on to the serde reader.
    fn forward_read_str<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'storage>;

    /// Transfer ownership of the next `length` bytes to the caller.
    fn get_byte_buffer(&mut self, length: usize) -> Result<Vec<u8>>;

    /// Pass a slice of the next `length` bytes on to the serde reader.
    fn forward_read_bytes<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'storage>;
}

/// A BincodeRead implementation for byte slices
/// NOT A PART OF THE STABLE PUBLIC API
#[doc(hidden)]
pub struct SliceReader<'storage> {
    slice: &'storage [u8],
}

/// A BincodeRead implementation for io::Readers
/// NOT A PART OF THE STABLE PUBLIC API
#[doc(hidden)]
pub struct IoReader<R> {
    reader: R,
    temp_buffer: Vec<u8>,
}

impl<'storage> SliceReader<'storage> {
    /// Constructs a slice reader
    pub fn new(bytes: &'storage [u8]) -> SliceReader<'storage> {
        SliceReader { slice: bytes }
    }

    #[inline(always)]
    fn get_byte_slice(&mut self, length: usize) -> Result<&'storage [u8]> {
        if length > self.slice.len() {
            return Err(SliceReader::unexpected_eof());
        }

        let s = &self.slice[..length];
        self.slice = &self.slice[length..];
        Ok(s)
    }
}

impl<R> IoReader<R> {
    /// Constructs an IoReadReader
    pub fn new(r: R) -> IoReader<R> {
        IoReader {
            reader: r,
            temp_buffer: vec![],
        }
    }
}

impl<'storage> io::Read for SliceReader<'storage> {
    #[inline(always)]
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        (&mut self.slice).read(out)
    }
    #[inline(always)]
    fn read_exact(&mut self, out: &mut [u8]) -> io::Result<()> {
        (&mut self.slice).read_exact(out)
    }
}

impl<R: io::Read> io::Read for IoReader<R> {
    #[inline(always)]
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.reader.read(out)
    }
    #[inline(always)]
    fn read_exact(&mut self, out: &mut [u8]) -> io::Result<()> {
        self.reader.read_exact(out)
    }
}

impl<'storage> SliceReader<'storage> {
    #[inline(always)]
    fn unexpected_eof() -> Box<::ErrorKind> {
        return Box::new(::ErrorKind::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "",
        )));
    }
}

impl<'storage> BincodeRead<'storage> for SliceReader<'storage> {
    #[inline(always)]
    fn forward_read_str<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'storage>,
    {
        use ErrorKind;
        let string = match ::std::str::from_utf8(self.get_byte_slice(length)?) {
            Ok(s) => s,
            Err(e) => return Err(ErrorKind::InvalidUtf8Encoding(e).into()),
        };
        visitor.visit_borrowed_str(string)
    }

    #[inline(always)]
    fn get_byte_buffer(&mut self, length: usize) -> Result<Vec<u8>> {
        self.get_byte_slice(length).map(|x| x.to_vec())
    }

    #[inline(always)]
    fn forward_read_bytes<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'storage>,
    {
        visitor.visit_borrowed_bytes(self.get_byte_slice(length)?)
    }
}

impl<R> IoReader<R>
where
    R: io::Read,
{
    fn fill_buffer(&mut self, length: usize) -> Result<()> {
        // We first reserve the space needed in our buffer.
        let current_length = self.temp_buffer.len();
        if length > current_length {
            self.temp_buffer.reserve_exact(length - current_length);
        }

        // Then create a slice with the length as our desired length. This is
        // safe as long as we only write (no reads) to this buffer, because
        // `reserve_exact` above has allocated this space.
        let buf = unsafe {
            slice::from_raw_parts_mut(self.temp_buffer.as_mut_ptr(), length)
        };

        // This method is assumed to properly handle slices which include
        // uninitialized bytes (as ours does). See discussion at the link below.
        // https://github.com/servo/bincode/issues/260
        self.reader.read_exact(buf)?;

        // Only after `read_exact` successfully returns do we set the buffer
        // length. By doing this after the call to `read_exact`, we can avoid
        // exposing uninitialized memory in the case of `read_exact` returning
        // an error.
        unsafe {
            self.temp_buffer.set_len(length);
        }

        Ok(())
    }
}

impl<'a, R> BincodeRead<'a> for IoReader<R>
where
    R: io::Read,
{
    fn forward_read_str<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'a>,
    {
        self.fill_buffer(length)?;

        let string = match ::std::str::from_utf8(&self.temp_buffer[..]) {
            Ok(s) => s,
            Err(e) => return Err(::ErrorKind::InvalidUtf8Encoding(e).into()),
        };

        visitor.visit_str(string)
    }

    fn get_byte_buffer(&mut self, length: usize) -> Result<Vec<u8>> {
        self.fill_buffer(length)?;
        Ok(::std::mem::replace(&mut self.temp_buffer, Vec::new()))
    }

    fn forward_read_bytes<V>(&mut self, length: usize, visitor: V) -> Result<V::Value>
    where
        V: serde::de::Visitor<'a>,
    {
        self.fill_buffer(length)?;
        visitor.visit_bytes(&self.temp_buffer[..])
    }
}
