//! A global allocator that counts, for the heap metrics: allocations, bytes allocated, and the
//! peak of live bytes.
//!
//! This is the only unsafe code in the workspace. Implementing `GlobalAlloc` needs it, and each
//! method only forwards to `System` with the caller's arguments; nothing else touches the
//! pointers. The tool isn't shipped, and the crates it measures stay free of unsafe code.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub struct Counting;

/// Calls to `alloc`, `alloc_zeroed`, and `realloc`.
static COUNT: AtomicU64 = AtomicU64::new(0);
/// Bytes asked for by those calls; a `realloc` counts its new size.
static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

fn grow(size: usize) {
    let live = LIVE.fetch_add(size as u64, Relaxed) + size as u64;
    PEAK.fetch_max(live, Relaxed);
}

fn shrink(size: usize) {
    LIVE.fetch_sub(size as u64, Relaxed);
}

fn count(size: usize) {
    COUNT.fetch_add(1, Relaxed);
    ALLOCATED.fetch_add(size as u64, Relaxed);
}

// SAFETY: every method forwards to `System`, which upholds `GlobalAlloc`'s contract, with the
// caller's arguments unchanged; the counters don't affect the memory returned.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded from the caller.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            count(layout.size());
            grow(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded from the caller.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            count(layout.size());
            grow(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded from the caller.
        unsafe { System.dealloc(ptr, layout) };
        shrink(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded from the caller.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            count(new_size);
            grow(new_size);
            shrink(layout.size());
        }
        new
    }
}

/// The counters at one moment.
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub count: u64,
    pub allocated: u64,
    pub live: u64,
    pub peak: u64,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        count: COUNT.load(Relaxed),
        allocated: ALLOCATED.load(Relaxed),
        live: LIVE.load(Relaxed),
        peak: PEAK.load(Relaxed),
    }
}

/// Starts a new peak from the bytes live now.
pub fn reset_peak() {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
}
