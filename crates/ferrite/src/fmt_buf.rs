//! Fixed-size, NUL-terminated format buffer — the no-allocation path for
//! turning `format_args!` into a C string for `app_log`.

pub(crate) struct FixedBuf {
    buf: [u8; 128],
    len: usize,
}

impl FixedBuf {
    pub(crate) fn new() -> FixedBuf {
        FixedBuf {
            buf: [0; 128],
            len: 0,
        }
    }

    /// NUL-terminated view; valid C string.
    pub(crate) fn as_cstr_ptr(&mut self) -> *const core::ffi::c_char {
        self.buf[self.len] = 0;
        self.buf.as_ptr().cast()
    }
}

impl core::fmt::Write for FixedBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let space = self.buf.len() - 1 - self.len; // reserve NUL
        let mut n = s.len().min(space);
        // Truncate on a UTF-8 character boundary so a multi-byte sequence is
        // never split. The test is on the first byte NOT copied: if it is a
        // continuation byte (0b10xx_xxxx) the cut lands mid-character, so back
        // up. When n == s.len() nothing is dropped and there is nothing to fix.
        while n > 0 && n < s.len() && (s.as_bytes()[n] & 0b1100_0000) == 0b1000_0000 {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FixedBuf;
    use core::fmt::Write;

    fn new() -> FixedBuf {
        FixedBuf {
            buf: [0; 128],
            len: 0,
        }
    }

    fn written(b: &FixedBuf) -> &[u8] {
        &b.buf[..b.len]
    }

    /// The buffer must never hold a partial UTF-8 sequence, and must keep the
    /// longest valid prefix that fits. Regression: an earlier version tested
    /// the last byte *copied* instead of the first byte *not* copied, which
    /// chopped the tail off characters that fit entirely.
    #[test]
    fn keeps_longest_valid_prefix_at_every_cut() {
        let samples = [
            "",
            "a",
            "hello",
            "é",
            "café",
            "日本語",
            "aé",
            "🦀",
            "a🦀b",
            "pan\u{00ed}c at src/lib.rs:1",
            "\u{10FFFF}\u{10FFFF}",
        ];
        for s in samples {
            for prefill in 0..127usize {
                let mut b = new();
                b.len = prefill;
                b.write_str(s).unwrap();
                let out = &b.buf[prefill..b.len];

                assert!(
                    core::str::from_utf8(out).is_ok(),
                    "invalid UTF-8 for {s:?} at prefill {prefill}: {out:?}"
                );
                // Must be the longest char-boundary prefix that fits.
                let space = 127 - prefill;
                let mut want = s.len().min(space);
                while !s.is_char_boundary(want) {
                    want -= 1;
                }
                assert_eq!(
                    out,
                    &s.as_bytes()[..want],
                    "wrong truncation for {s:?} at prefill {prefill}"
                );
            }
        }
    }

    #[test]
    fn never_overruns_or_fills_the_reserved_nul_slot() {
        let mut b = new();
        for _ in 0..100 {
            let _ = b.write_str("0123456789");
        }
        // One byte is reserved so the caller can always NUL-terminate.
        assert!(b.len < b.buf.len(), "len {} overran", b.len);
        b.buf[b.len] = 0; // must be in bounds, exactly as the handler does it
        assert_eq!(b.buf[b.len], 0);
    }

    #[test]
    fn write_str_appends_across_calls() {
        let mut b = new();
        b.write_str("ab").unwrap();
        b.write_str("cd").unwrap();
        assert_eq!(written(&b), b"abcd");
    }

    #[test]
    fn multibyte_char_is_dropped_whole_when_it_cannot_fit() {
        let mut b = new();
        b.len = 126; // 1 byte of usable space, reserving the NUL slot
        b.write_str("é").unwrap(); // 2 bytes -> must not be split
        assert_eq!(b.len, 126, "a 2-byte char was split into 1 byte");
    }
}
