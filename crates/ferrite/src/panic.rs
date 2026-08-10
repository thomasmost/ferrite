//! Panic = log via app_log, then trap so the firmware's app-fault path
//! terminates the app. There is no unwinding (panic = "abort").

#[cfg(target_os = "none")]
mod panic_impl {
    use core::fmt::Write;

    use crate::fmt_buf::FixedBuf;

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut out = FixedBuf::new();
        let _ = write!(out, "{}", info);
        unsafe {
            crate::sys::app_log(
                crate::sys::AppLogLevel::APP_LOG_LEVEL_ERROR.0,
                c"rust".as_ptr(),
                0,
                c"%s".as_ptr(),
                out.as_cstr_ptr(),
            );
        }
        // Undefined instruction: firmware kills the app through its fault handler.
        loop {
            unsafe { core::arch::asm!("udf #255", options(nomem, nostack)) };
        }
    }
}
