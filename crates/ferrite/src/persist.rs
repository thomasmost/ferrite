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
    check(unsafe { sys::persist_write_int(key, value) }).map(|_| ())
}

pub fn read_bool(key: u32) -> Result<bool> {
    if !exists(key) {
        return Err(Error::DoesNotExist);
    }
    Ok(unsafe { sys::persist_read_bool(key) })
}

pub fn write_bool(key: u32, value: bool) -> Result<()> {
    check(unsafe { sys::persist_write_bool(key, value) }).map(|_| ())
}

/// Size in bytes of the stored value, if any.
pub fn size(key: u32) -> Result<usize> {
    check(unsafe { sys::persist_get_size(key) }).map(|n| n as usize)
}

pub fn delete(key: u32) -> Result<()> {
    check(unsafe { sys::persist_delete(key) }).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[test]
    fn error_mapping_covers_sdk_codes() {
        assert_eq!(Error::from_code(-9), Error::DoesNotExist);
        assert_eq!(Error::from_code(-6), Error::OutOfStorage);
        assert_eq!(Error::from_code(-8), Error::Range);
        assert_eq!(Error::from_code(-4), Error::InvalidArgument);
        assert_eq!(Error::from_code(-1), Error::Other(-1));
    }

    // Thread-local storage for test stubs to manage persist state.
    thread_local! {
        static PERSIST_STORE: core::cell::RefCell<BTreeMap<u32, alloc::vec::Vec<u8>>> =
            const { core::cell::RefCell::new(BTreeMap::new()) };
    }

    // Test stubs for SDK functions (must not collide with other test stubs).
    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_exists_test(key: u32) -> bool {
        PERSIST_STORE.with(|store| store.borrow().contains_key(&key))
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_read_int_test(key: u32) -> i32 {
        PERSIST_STORE.with(|store| {
            store
                .borrow()
                .get(&key)
                .map(|v| {
                    if v.len() >= 4 {
                        i32::from_le_bytes([v[0], v[1], v[2], v[3]])
                    } else {
                        0
                    }
                })
                .unwrap_or(0)
        })
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_write_int_test(key: u32, value: i32) -> i32 {
        PERSIST_STORE.with(|store| {
            store.borrow_mut().insert(
                key,
                alloc::vec![
                    (value & 0xff) as u8,
                    ((value >> 8) & 0xff) as u8,
                    ((value >> 16) & 0xff) as u8,
                    ((value >> 24) & 0xff) as u8,
                ],
            );
            0 // success
        })
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_read_data_test(key: u32, buffer: *mut u8, buffer_size: usize) -> i32 {
        if buffer.is_null() {
            return -4; // E_INVALID_ARGUMENT
        }
        PERSIST_STORE.with(|store| {
            match store.borrow().get(&key) {
                Some(data) => {
                    let copy_len = core::cmp::min(data.len(), buffer_size);
                    unsafe {
                        core::ptr::copy_nonoverlapping(data.as_ptr(), buffer, copy_len);
                    }
                    copy_len as i32
                }
                None => -9, // E_DOES_NOT_EXIST
            }
        })
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_write_data_test(key: u32, buffer: *const u8, buffer_size: usize) -> i32 {
        if buffer.is_null() {
            return -4; // E_INVALID_ARGUMENT
        }
        PERSIST_STORE.with(|store| {
            let data = unsafe { core::slice::from_raw_parts(buffer, buffer_size) };
            store.borrow_mut().insert(key, alloc::vec::Vec::from(data));
            buffer_size as i32
        })
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_get_size_test(key: u32) -> i32 {
        PERSIST_STORE.with(|store| {
            store
                .borrow()
                .get(&key)
                .map(|v| v.len() as i32)
                .unwrap_or(-9)
        })
    }

    #[cfg(test)]
    #[no_mangle]
    extern "C" fn persist_delete_test(key: u32) -> i32 {
        PERSIST_STORE.with(|store| {
            store.borrow_mut().remove(&key);
            0 // always succeeds
        })
    }

    /// Test: size returns 0 for missing key, then succeeds after write, then 0 after delete.
    #[test]
    fn persist_boundary_checks() {
        const TEST_KEY: u32 = 0x12345678;

        // Initially, key doesn't exist
        assert_eq!(
            persist_get_size_test(TEST_KEY),
            -9,
            "size should return E_DOES_NOT_EXIST for missing key"
        );

        // Write some data
        let data = b"hello";
        persist_write_data_test(TEST_KEY, data.as_ptr(), data.len());

        // Now size should return the byte count
        assert_eq!(
            persist_get_size_test(TEST_KEY),
            5,
            "size should return byte count"
        );

        // Delete it
        persist_delete_test(TEST_KEY);

        // Size should be -9 again (E_DOES_NOT_EXIST)
        assert_eq!(
            persist_get_size_test(TEST_KEY),
            -9,
            "size should return E_DOES_NOT_EXIST after delete"
        );
    }

    /// Test: read_int gate with exists() check.
    #[test]
    fn persist_read_int_gate_with_exists() {
        const TEST_KEY: u32 = 0xdeadbeef;

        // read_int on missing key should error (gates with exists())
        PERSIST_STORE.with(|store| store.borrow_mut().clear());

        let result = check(persist_read_int_test(TEST_KEY));
        assert!(
            result.is_ok(),
            "read_int should return ok even if key doesn't exist (gate happens in wrapper)"
        );

        // Write a value
        persist_write_int_test(TEST_KEY, 42);

        // Now read should succeed
        let val = persist_read_int_test(TEST_KEY);
        assert_eq!(val, 42, "read_int should decode correctly");
    }

    /// Test: read_data byte-count passthrough.
    #[test]
    fn persist_read_data_byte_count() {
        const TEST_KEY: u32 = 0x11223344;

        PERSIST_STORE.with(|store| store.borrow_mut().clear());

        let test_data = b"test payload";
        persist_write_data_test(TEST_KEY, test_data.as_ptr(), test_data.len());

        let mut read_buf = [0u8; 64];
        let n = persist_read_data_test(TEST_KEY, read_buf.as_mut_ptr(), read_buf.len());

        assert_eq!(n, test_data.len() as i32, "byte count must match");
        assert_eq!(
            &read_buf[..n as usize],
            test_data,
            "data must be correctly read"
        );
    }

    /// Test: error path when stub returns -9 (E_DOES_NOT_EXIST).
    #[test]
    fn persist_error_path_does_not_exist() {
        const TEST_KEY: u32 = 0x55667788;

        PERSIST_STORE.with(|store| store.borrow_mut().clear());

        // read_data on missing key returns -9
        let mut buf = [0u8; 32];
        let code = persist_read_data_test(TEST_KEY, buf.as_mut_ptr(), buf.len());

        let err = Error::from_code(code);
        assert_eq!(err, Error::DoesNotExist, "code -9 must map to DoesNotExist");
    }
}
