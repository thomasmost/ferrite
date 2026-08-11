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
        // Pinned to the bindings' StatusCode constants, not transcribed
        // literals: if a future `cargo xtask bindgen` run ever moves a status
        // code, this keeps mapping the SDK's actual values instead of
        // silently degrading every branch to Error::Other.
        match code {
            c if c == sys::StatusCode::E_DOES_NOT_EXIST.0 as i32 => Error::DoesNotExist,
            c if c == sys::StatusCode::E_OUT_OF_STORAGE.0 as i32 => Error::OutOfStorage,
            c if c == sys::StatusCode::E_RANGE.0 as i32 => Error::Range,
            c if c == sys::StatusCode::E_INVALID_ARGUMENT.0 as i32 => Error::InvalidArgument,
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

    // Real-symbol SDK stubs: the host linker uses these #[no_mangle]
    // definitions to satisfy the bindgen `extern` declarations, so the tests
    // below exercise the REAL public wrappers (exists/read/write/size/delete)
    // and the REAL check()/Error mapping -- not a parallel mock API. House
    // pattern per click.rs::provider_tests.
    //
    // Behaviour is scripted per-test through thread_locals (each test's
    // stubs read its own thread's state, so parallel tests cannot interfere).
    thread_local! {
        static EXISTS: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
        static INT_VALUE: core::cell::Cell<i32> = const { core::cell::Cell::new(0) };
        static RET_CODE: core::cell::Cell<i32> = const { core::cell::Cell::new(0) };
        static DATA: core::cell::RefCell<alloc::vec::Vec<u8>> =
            const { core::cell::RefCell::new(alloc::vec::Vec::new()) };
    }

    #[no_mangle]
    extern "C" fn persist_exists(_key: u32) -> bool {
        EXISTS.with(|c| c.get())
    }

    #[no_mangle]
    extern "C" fn persist_read_int(_key: u32) -> i32 {
        INT_VALUE.with(|c| c.get())
    }

    #[no_mangle]
    extern "C" fn persist_write_int(_key: u32, value: i32) -> sys::status_t {
        INT_VALUE.with(|c| c.set(value));
        RET_CODE.with(|c| c.get())
    }

    #[no_mangle]
    extern "C" fn persist_read_bool(_key: u32) -> bool {
        INT_VALUE.with(|c| c.get()) != 0
    }

    #[no_mangle]
    extern "C" fn persist_write_bool(_key: u32, value: bool) -> sys::status_t {
        INT_VALUE.with(|c| c.set(value as i32));
        RET_CODE.with(|c| c.get())
    }

    #[no_mangle]
    extern "C" fn persist_read_data(
        _key: u32,
        buffer: *mut core::ffi::c_void,
        buffer_size: usize,
    ) -> core::ffi::c_int {
        let code = RET_CODE.with(|c| c.get());
        if code < 0 {
            return code;
        }
        DATA.with(|d| {
            let d = d.borrow();
            let n = d.len().min(buffer_size);
            unsafe {
                core::ptr::copy_nonoverlapping(d.as_ptr(), buffer as *mut u8, n);
            }
            n as core::ffi::c_int
        })
    }

    #[no_mangle]
    extern "C" fn persist_write_data(
        _key: u32,
        data: *const core::ffi::c_void,
        size: usize,
    ) -> core::ffi::c_int {
        let code = RET_CODE.with(|c| c.get());
        if code < 0 {
            return code;
        }
        DATA.with(|d| {
            let mut d = d.borrow_mut();
            d.clear();
            d.extend_from_slice(unsafe { core::slice::from_raw_parts(data as *const u8, size) });
        });
        size as core::ffi::c_int
    }

    #[no_mangle]
    extern "C" fn persist_get_size(_key: u32) -> core::ffi::c_int {
        let code = RET_CODE.with(|c| c.get());
        if code < 0 {
            return code;
        }
        DATA.with(|d| d.borrow().len() as core::ffi::c_int)
    }

    #[no_mangle]
    extern "C" fn persist_delete(_key: u32) -> sys::status_t {
        RET_CODE.with(|c| c.get())
    }

    #[test]
    fn error_mapping_covers_sdk_codes() {
        assert_eq!(Error::from_code(-9), Error::DoesNotExist);
        assert_eq!(Error::from_code(-6), Error::OutOfStorage);
        assert_eq!(Error::from_code(-8), Error::Range);
        assert_eq!(Error::from_code(-4), Error::InvalidArgument);
        assert_eq!(Error::from_code(-1), Error::Other(-1));
    }

    /// The exists() gate is the reason read_int/read_bool wrappers exist:
    /// the raw SDK returns 0 for a missing key, indistinguishable from a
    /// stored 0. Through the REAL wrapper, a missing key must be an error.
    #[test]
    fn read_int_missing_key_is_does_not_exist() {
        EXISTS.with(|c| c.set(false));
        INT_VALUE.with(|c| c.set(42)); // would be returned if the gate broke
        assert_eq!(read_int(7), Err(Error::DoesNotExist));

        EXISTS.with(|c| c.set(true));
        assert_eq!(read_int(7), Ok(42));
    }

    #[test]
    fn read_bool_missing_key_is_does_not_exist() {
        EXISTS.with(|c| c.set(false));
        assert_eq!(read_bool(7), Err(Error::DoesNotExist));
        EXISTS.with(|c| c.set(true));
        INT_VALUE.with(|c| c.set(1));
        assert_eq!(read_bool(7), Ok(true));
    }

    /// check(0) is a boundary: persist_delete returns S_TRUE/S_FALSE (>= 0)
    /// and must be Ok, not an error.
    #[test]
    fn check_zero_is_ok_through_delete() {
        RET_CODE.with(|c| c.set(0));
        assert_eq!(delete(7), Ok(()));
    }

    /// Negative SDK codes must round-trip through check() into Error --
    /// through the REAL public wrappers, not the mapping helper alone.
    #[test]
    fn negative_codes_map_to_errors_through_real_calls() {
        RET_CODE.with(|c| c.set(-9));
        assert_eq!(delete(7), Err(Error::DoesNotExist));
        RET_CODE.with(|c| c.set(-6));
        assert_eq!(write_data(7, b"x"), Err(Error::OutOfStorage));
        RET_CODE.with(|c| c.set(-8));
        assert_eq!(write_int(7, 1), Err(Error::Range));
        RET_CODE.with(|c| c.set(0));
    }

    /// write_int and write_bool succeed through the real call path (the Ok
    /// branch of check(), and the only coverage write_bool has).
    #[test]
    fn int_and_bool_writes_succeed() {
        RET_CODE.with(|c| c.set(4));
        assert_eq!(write_int(7, 123), Ok(()));
        assert_eq!(write_bool(7, true), Ok(()));
        EXISTS.with(|c| c.set(true));
        assert_eq!(read_int(7), Ok(1)); // write_bool stored last: true as 1
        RET_CODE.with(|c| c.set(0));
    }

    /// Byte counts pass through unchanged, and read_data round-trips what
    /// write_data stored.
    #[test]
    fn data_round_trip_and_byte_counts() {
        RET_CODE.with(|c| c.set(0));
        assert_eq!(write_data(7, b"hello"), Ok(5));
        assert_eq!(size(7), Ok(5));

        let mut buf = [0u8; 16];
        assert_eq!(read_data(7, &mut buf), Ok(5));
        assert_eq!(&buf[..5], b"hello");
    }
}
