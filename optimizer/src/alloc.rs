//! A thread-caching allocator for small blocks. musl's malloc takes one global lock, which made 12 search threads
//! run ~10x slower than one; small blocks (<= 1 KiB) are served from per-thread free lists instead (memory carved
//! from 256 KiB chunks and never returned to the system; blocks freed on another thread join that thread's lists).
//! A thread that ends hands its lists to a shared pool the other threads refill from, so threads started and ended
//! again and again (the LNS / ILS rounds) reuse that memory instead of losing it.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;

const STEP: usize = 16;
const CLASSES: usize = 64; // 16 ..= 1024 bytes
const CHUNK: usize = 256 * 1024;

struct Local {
    free: [*mut u8; CLASSES],
}

/// Free lists of ended threads (heads as addresses), one per size class.
static ORPHANS: std::sync::Mutex<[usize; CLASSES]> = std::sync::Mutex::new([0; CLASSES]);

impl Drop for Local {
    fn drop(&mut self) {
        let Ok(mut o) = ORPHANS.lock() else { return };
        for c in 0..CLASSES {
            let head = self.free[c];
            if head.is_null() {
                continue;
            }
            // append the pool's list after this list's last block
            unsafe {
                let mut tail = head;
                loop {
                    let next = *(tail as *mut *mut u8);
                    if next.is_null() {
                        break;
                    }
                    tail = next;
                }
                *(tail as *mut *mut u8) = o[c] as *mut u8;
            }
            o[c] = head as usize;
            self.free[c] = std::ptr::null_mut();
        }
    }
}

thread_local! {
    static L: UnsafeCell<Local> = const { UnsafeCell::new(Local { free: [std::ptr::null_mut(); CLASSES] }) };
}

pub struct Fast;

#[inline]
fn class_of(l: &Layout) -> Option<usize> {
    if l.size() <= CLASSES * STEP && l.align() <= STEP {
        Some((l.size().max(1) + STEP - 1) / STEP - 1)
    } else {
        None
    }
}

unsafe fn refill(loc: &mut Local, c: usize) -> *mut u8 {
    // an ended thread's list first
    if let Ok(mut o) = ORPHANS.try_lock() {
        let head = o[c] as *mut u8;
        if !head.is_null() {
            o[c] = 0;
            loc.free[c] = *(head as *mut *mut u8);
            return head;
        }
    }
    let size = (c + 1) * STEP;
    let chunk = System.alloc(Layout::from_size_align_unchecked(CHUNK, STEP));
    if chunk.is_null() {
        return chunk;
    }
    let n = CHUNK / size;
    // block 0 is returned; blocks 1..n go on the free list
    for i in 1..n {
        let b = chunk.add(i * size);
        let next = if i + 1 < n { chunk.add((i + 1) * size) } else { loc.free[c] };
        *(b as *mut *mut u8) = next;
    }
    if n > 1 {
        loc.free[c] = chunk.add(size);
    }
    chunk
}

unsafe impl GlobalAlloc for Fast {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let Some(c) = class_of(&l) else { return System.alloc(l) };
        match L.try_with(|x| {
            let loc = &mut *x.get();
            let p = loc.free[c];
            if p.is_null() {
                refill(loc, c)
            } else {
                loc.free[c] = *(p as *mut *mut u8);
                p
            }
        }) {
            Ok(p) => p,
            Err(_) => System.alloc(Layout::from_size_align_unchecked((c + 1) * STEP, STEP)),
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let Some(c) = class_of(&l) else { return System.dealloc(p, l) };
        let _ = L.try_with(|x| {
            let loc = &mut *x.get();
            *(p as *mut *mut u8) = loc.free[c];
            loc.free[c] = p;
        });
        // during thread teardown the block is leaked (it came from a chunk, not from System directly)
    }
}
