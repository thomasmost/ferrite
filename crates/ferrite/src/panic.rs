//! Panic = log via app_log, then trap so the firmware's app-fault path
//! terminates the app. There is no unwinding (panic = "abort").

use core::fmt::Write;

struct FixedBuf {
    buf: [u8; 128],
    len: usize,
}

impl Write for FixedBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let space = self.buf.len() - 1 - self.len; // reserve NUL byte
        let mut n = s.len().min(space);
        // Truncate on a UTF-8 character boundary to avoid splitting multi-byte sequences
        while n > 0 && (s.as_bytes()[n - 1] & 0b11000000) == 0b10000000 {
            n -= 1; // back up over continuation bytes
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut out = FixedBuf {
        buf: [0; 128],
        len: 0,
    };
    let _ = write!(out, "{}", info);
    out.buf[out.len] = 0;
    unsafe {
        crate::sys::app_log(
            crate::sys::APP_LOG_LEVEL_ERROR,
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            out.buf.as_ptr(),
        );
    }
    // Undefined instruction: firmware kills the app through its fault handler.
    loop {
        unsafe { core::arch::asm!("udf #255", options(nomem, nostack)) };
    }
}
