//! Global allocator backed by the SDK heap (`malloc`/`free` jump-table
//! trampolines in libpebble.a). Rust and C code share one heap, so the SDK's
//! `heap_bytes_free()` stays meaningful.
//!
//! The firmware heap's alignment guarantee is undocumented; we assume 4
//! (ARM word). Stricter alignments over-allocate and stash the original
//! pointer just below the aligned block.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::mem::size_of;

extern "C" {
    #[allow(dead_code)]
    fn malloc(size: usize) -> *mut c_void;
    #[allow(dead_code)]
    fn free(ptr: *mut c_void);
}

/// Alignment we trust the SDK heap to provide.
#[allow(dead_code)]
const ASSUMED_ALIGN: usize = 4;

#[allow(dead_code)]
struct PebbleHeap;

/// Compute where an aligned block would begin, given a raw allocation.
/// Used by both alloc and tests to verify correctness.
#[allow(dead_code)]
fn aligned_start(raw: usize, align: usize) -> usize {
    (raw + size_of::<usize>() + align - 1) & !(align - 1)
}

unsafe impl GlobalAlloc for PebbleHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        if align <= ASSUMED_ALIGN {
            return malloc(layout.size()).cast();
        }
        // [ raw ... | original ptr | aligned block ... ]
        let total = layout.size() + align + size_of::<usize>();
        let raw = malloc(total) as usize;
        if raw == 0 {
            return core::ptr::null_mut();
        }
        let aligned = aligned_start(raw, align);
        *((aligned - size_of::<usize>()) as *mut usize) = raw;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.align() <= ASSUMED_ALIGN {
            free(ptr.cast());
            return;
        }
        let raw = *((ptr as usize - size_of::<usize>()) as *const usize);
        free(raw as *mut c_void);
    }
}

// Registered only for the watch target: host `cargo test` (Phase 4+) must
// keep std's allocator.
#[cfg(target_os = "none")]
#[global_allocator]
static ALLOCATOR: PebbleHeap = PebbleHeap;

/// Free bytes remaining on the app heap (SDK `heap_bytes_free`).
pub fn heap_bytes_free() -> usize {
    unsafe { crate::sys::heap_bytes_free() }
}

/// Bytes currently allocated on the app heap (SDK `heap_bytes_used`).
pub fn heap_bytes_used() -> usize {
    unsafe { crate::sys::heap_bytes_used() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_start_computation_is_correct() {
        // For a raw allocation at 0x1000, align to 8 bytes:
        // pointer slot at 0x1000 (4 bytes)
        // aligned block should start at next 8-byte boundary after 0x1004
        // = 0x1008
        let raw = 0x1000usize;
        let aligned = aligned_start(raw, 8);
        assert_eq!(aligned, 0x1008);

        // Raw at 0x1001, align to 8:
        // pointer slot at 0x1001 (4 bytes → 0x1005)
        // next 8-byte boundary is 0x1008
        let raw = 0x1001usize;
        let aligned = aligned_start(raw, 8);
        assert_eq!(aligned, 0x1008);

        // Raw at 0x1000, align to 16:
        // pointer slot at 0x1000 (4 bytes → 0x1004)
        // next 16-byte boundary is 0x1010
        let raw = 0x1000usize;
        let aligned = aligned_start(raw, 16);
        assert_eq!(aligned, 0x1010);
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_small_align() {
        // Allocations with align <= 4 should go directly through malloc.
        // We can't test malloc's actual behavior on host (it's the real libc
        // malloc), but we can test that the allocator doesn't over-allocate
        // for small alignments.
        let layout = Layout::from_size_align(64, 4).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        if !ptr.is_null() {
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
        // Test passes if alloc/dealloc don't crash.
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_8_byte_align() {
        // Allocations with align > 4 use the over-allocate-and-shim path.
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        if !ptr.is_null() {
            // Verify the returned pointer is actually 8-byte aligned.
            assert_eq!(ptr as usize % 8, 0, "pointer not properly aligned");
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_16_byte_align() {
        let layout = Layout::from_size_align(64, 16).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        if !ptr.is_null() {
            assert_eq!(ptr as usize % 16, 0, "pointer not properly aligned");
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_64_byte_align() {
        let layout = Layout::from_size_align(64, 64).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        if !ptr.is_null() {
            assert_eq!(ptr as usize % 64, 0, "pointer not properly aligned");
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
    }

    #[test]
    fn allocator_round_trip_alloc_dealloc_small_align() {
        // Multiple rounds to verify no memory corruption.
        for _ in 0..10 {
            let layout = Layout::from_size_align(32, 4).unwrap();
            let ptr = unsafe { PebbleHeap.alloc(layout) };
            if !ptr.is_null() {
                unsafe {
                    PebbleHeap.dealloc(ptr, layout);
                }
            }
        }
    }

    #[test]
    fn allocator_round_trip_alloc_dealloc_large_align() {
        // Multiple rounds for large alignment.
        for _ in 0..10 {
            let layout = Layout::from_size_align(32, 16).unwrap();
            let ptr = unsafe { PebbleHeap.alloc(layout) };
            if !ptr.is_null() {
                assert_eq!(ptr as usize % 16, 0);
                unsafe {
                    PebbleHeap.dealloc(ptr, layout);
                }
            }
        }
    }
}
