//! Persistent key-value storage, wrapping the SDK `persist_*` API with
//! `Result`s. Values are capped at `PERSIST_DATA_MAX_LENGTH` (256) bytes.

use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// No value stored under this key (`E_DOES_NOT_EXIST`).
    DoesNotExist,
    /// Storage quota exhausted (`E_OUT_OF_STORAGE`).
    OutOfStorage,
    /// Value or buffer out of range (`E_RANGE`).
    Range,
    /// Bad argument (`E_INVALID_ARGUMENT`).
    InvalidArgument,
    /// Any other negative SDK status code.
    Other(i32),
}

impl Error {
    fn from_code(code: i32) -> Error {
        match code {
            -9 => Error::DoesNotExist,
            -6 => Error::OutOfStorage,
            -8 => Error::Range,
            -4 => Error::InvalidArgument,
            other => Error::Other(other),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

fn check(code: i32) -> Result<i32> {
    if code < 0 {
        Err(Error::from_code(code))
    } else {
        Ok(code)
    }
}

pub fn exists(key: u32) -> bool {
    unsafe { sys::persist_exists(key) }
}

/// Reads a blob into `buf`; returns the number of bytes read.
pub fn read_data(key: u32, buf: &mut [u8]) -> Result<usize> {
    check(unsafe { sys::persist_read_data(key, buf.as_mut_ptr().cast(), buf.len()) })
        .map(|n| n as usize)
}

/// Writes a blob (max 256 bytes); returns the number of bytes written.
pub fn write_data(key: u32, data: &[u8]) -> Result<usize> {
    check(unsafe { sys::persist_write_data(key, data.as_ptr().cast(), data.len()) })
        .map(|n| n as usize)
}

pub fn read_int(key: u32) -> Result<i32> {
    if !exists(key) {
        return Err(Error::DoesNotExist);
    }
    Ok(unsafe { sys::persist_read_int(key) })
}

pub fn write_int(key: u32, value: i32) -> Result<()> {
    check(unsafe { sys::persist_write_int(key, value) } as i32).map(|_| ())
}

pub fn read_bool(key: u32) -> Result<bool> {
    if !exists(key) {
        return Err(Error::DoesNotExist);
    }
    Ok(unsafe { sys::persist_read_bool(key) })
}

pub fn write_bool(key: u32, value: bool) -> Result<()> {
    check(unsafe { sys::persist_write_bool(key, value) } as i32).map(|_| ())
}

/// Size in bytes of the stored value, if any.
pub fn size(key: u32) -> Result<usize> {
    check(unsafe { sys::persist_get_size(key) }).map(|n| n as usize)
}

pub fn delete(key: u32) -> Result<()> {
    check(unsafe { sys::persist_delete(key) } as i32).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_mapping_covers_sdk_codes() {
        assert_eq!(Error::from_code(-9), Error::DoesNotExist);
        assert_eq!(Error::from_code(-6), Error::OutOfStorage);
        assert_eq!(Error::from_code(-8), Error::Range);
        assert_eq!(Error::from_code(-4), Error::InvalidArgument);
        assert_eq!(Error::from_code(-1), Error::Other(-1));
    }
}
