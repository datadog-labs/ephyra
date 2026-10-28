//! Dropping a BBTreeMap while a value's Drop panics leaks the pages on its free lists.
use std::alloc::Layout;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use bbtree::{AllocError, Allocator, Auto, BBTreeMap, Global};

/// The global allocator, counting the pages out.
static PAGES_OUT: AtomicIsize = AtomicIsize::new(0);
struct Counting;
unsafe impl Allocator for Counting {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        PAGES_OUT.fetch_add(1, Ordering::SeqCst);
        Global.allocate_zeroed(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        PAGES_OUT.fetch_sub(1, Ordering::SeqCst);
        unsafe { Global.deallocate(ptr, layout) }
    }
}

/// A value whose Drop panics once armed.
static ARMED: AtomicBool = AtomicBool::new(false);
struct Bomb;
impl Drop for Bomb {
    fn drop(&mut self) {
        if ARMED.swap(false, Ordering::SeqCst) {
            panic!("value's Drop panics");
        }
    }
}

fn main() {
    let map = BBTreeMap::<u64, Bomb, Auto, Counting>::new_in(Counting);
    for k in 0..2000 {
        map.insert(k, Bomb).unwrap();
    }
    for k in 0..1990 {
        map.remove(&k); // emptied leaves go to the map's free lists
    }
    println!("pages out before drop: {}", PAGES_OUT.load(Ordering::SeqCst));
    ARMED.store(true, Ordering::SeqCst);
    let panicked = catch_unwind(AssertUnwindSafe(move || drop(map))).is_err();
    println!("drop panicked: {panicked}");
    println!("pages out after drop: {}", PAGES_OUT.load(Ordering::SeqCst));
}
