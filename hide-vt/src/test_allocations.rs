// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// crates/ghostty-vt/src/test_allocations.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Unchanged by Hide.

//! Count Rust allocation requests on only the calling test thread.
//! Native terminal allocations and concurrent tests are deliberately excluded.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn record_allocation() {
    // Thread-local storage may already be gone during thread teardown.
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub fn count<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.set(None);
        }
    }

    assert!(ALLOCATIONS.replace(Some(0)).is_none());
    let _reset = Reset;
    let result = operation();
    let count = ALLOCATIONS.get().expect("allocation counting is active");
    (result, count)
}
