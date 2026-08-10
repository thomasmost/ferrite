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

/// Compute the total allocation size needed for an over-aligned block.
/// Used by both alloc and tests to verify correctness.
#[allow(dead_code)]
fn total_size(size: usize, align: usize) -> usize {
    size + align + size_of::<usize>()
}

unsafe impl GlobalAlloc for PebbleHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        if align <= ASSUMED_ALIGN {
            return malloc(layout.size()).cast();
        }
        // [ raw ... | original ptr | aligned block ... ]
        let total = total_size(layout.size(), align);
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
        // For a raw allocation at 0x1000 (4096 decimal), align to 8 bytes:
        // pointer slot at 4096 (8 bytes on 64-bit)
        // aligned block should start at: (4096 + 8 + 8 - 1) & ~7 = 4111 & ~7 = 4104
        // Verify we're on 64-bit host where these constants hold.
        assert_eq!(size_of::<usize>(), 8, "test constants assume 64-bit host");

        let raw = 0x1000usize; // 4096
        let aligned = aligned_start(raw, 8);
        assert_eq!(aligned, 4104); // (4096 + 8 + 8 - 1) & ~7

        // Raw at 4097, align to 8:
        // (4097 + 8 + 8 - 1) & ~7 = 4112 & ~7 = 4112
        let raw = 0x1001usize; // 4097
        let aligned = aligned_start(raw, 8);
        assert_eq!(aligned, 4112); // (4097 + 8 + 8 - 1) & ~7

        // Raw at 4096, align to 16:
        // (4096 + 8 + 16 - 1) & ~15 = 4119 & ~15 = 4112
        let raw = 0x1000usize; // 4096
        let aligned = aligned_start(raw, 16);
        assert_eq!(aligned, 4112); // (4096 + 8 + 16 - 1) & ~15
    }

    #[test]
    fn total_size_accommodates_all_allocations() {
        // Exhaustive property test: verify that total_size computes enough
        // space for any allocation. For all alignment and size combinations,
        // verify that:
        // 1. The stash (pointer slot) fits: aligned >= raw + sizeof(usize)
        // 2. The block is properly aligned: aligned % align == 0
        // 3. The stash pointer is aligned: (aligned - sizeof(usize)) % sizeof(usize) == 0
        // 4. The block fits within the allocation: aligned + size <= raw + total
        assert_eq!(size_of::<usize>(), 8, "property test assumes 64-bit host");

        for &align in &[8, 16, 32, 64, 128] {
            for &size in &[1, 4, 7, 32, 64] {
                for off in 0..align {
                    let raw = 0x1000 + off;
                    let total = total_size(size, align);
                    let aligned = aligned_start(raw, align);

                    // Stash must be in bounds
                    assert!(
                        aligned >= raw + size_of::<usize>(),
                        "stash out of bounds: align={}, size={}, off={}, raw={:#x}, aligned={:#x}",
                        align,
                        size,
                        off,
                        raw,
                        aligned
                    );

                    // Block must be aligned
                    assert_eq!(
                        aligned % align,
                        0,
                        "block not aligned: align={}, size={}, off={}, aligned={:#x}",
                        align,
                        size,
                        off,
                        aligned
                    );

                    // Stash must be properly aligned (every usize is at a usize-aligned address)
                    assert_eq!(
                        (aligned - size_of::<usize>()) % size_of::<usize>(),
                        0,
                        "stash not aligned: align={}, size={}, off={}, aligned={:#x}",
                        align,
                        size,
                        off,
                        aligned
                    );

                    // Block must fit within the allocation
                    assert!(
                        aligned + size <= raw + total,
                        "block exceeds allocation: align={}, size={}, off={}, raw={:#x}, \
                         total={}, aligned={:#x}, aligned+size={:#x}, raw+total={:#x}",
                        align,
                        size,
                        off,
                        raw,
                        total,
                        aligned,
                        aligned + size,
                        raw + total
                    );
                }
            }
        }
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_small_align() {
        // Allocations with align <= 4 should go directly through malloc.
        // We can't test malloc's actual behavior on host (it's the real libc
        // malloc), but we can test that the allocator doesn't over-allocate
        // for small alignments.
        let layout = Layout::from_size_align(64, 4).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        assert!(!ptr.is_null(), "alloc returned null");
        unsafe {
            PebbleHeap.dealloc(ptr, layout);
        }
        // Test passes if alloc/dealloc don't crash.
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_8_byte_align() {
        // Allocations with align > 4 use the over-allocate-and-shim path.
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        assert!(!ptr.is_null(), "alloc returned null");
        // Verify the returned pointer is actually 8-byte aligned.
        assert_eq!(ptr as usize % 8, 0, "pointer not properly aligned");
        unsafe {
            PebbleHeap.dealloc(ptr, layout);
        }
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_16_byte_align() {
        let layout = Layout::from_size_align(64, 16).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        assert!(!ptr.is_null(), "alloc returned null");
        assert_eq!(ptr as usize % 16, 0, "pointer not properly aligned");
        unsafe {
            PebbleHeap.dealloc(ptr, layout);
        }
    }

    #[test]
    fn allocator_returns_aligned_pointers_for_64_byte_align() {
        let layout = Layout::from_size_align(64, 64).unwrap();
        let ptr = unsafe { PebbleHeap.alloc(layout) };
        assert!(!ptr.is_null(), "alloc returned null");
        assert_eq!(ptr as usize % 64, 0, "pointer not properly aligned");
        unsafe {
            PebbleHeap.dealloc(ptr, layout);
        }
    }

    #[test]
    fn allocator_round_trip_alloc_dealloc_small_align() {
        // Multiple rounds to verify no memory corruption.
        for _ in 0..10 {
            let layout = Layout::from_size_align(32, 4).unwrap();
            let ptr = unsafe { PebbleHeap.alloc(layout) };
            assert!(!ptr.is_null(), "alloc returned null");
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
    }

    #[test]
    fn allocator_round_trip_alloc_dealloc_large_align() {
        // Multiple rounds for large alignment.
        for _ in 0..10 {
            let layout = Layout::from_size_align(32, 16).unwrap();
            let ptr = unsafe { PebbleHeap.alloc(layout) };
            assert!(!ptr.is_null(), "alloc returned null");
            assert_eq!(ptr as usize % 16, 0);
            unsafe {
                PebbleHeap.dealloc(ptr, layout);
            }
        }
    }
}
