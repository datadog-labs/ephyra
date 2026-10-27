use bbtree::{Allocator, AllocError, BBTreeMap};
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicIsize, Ordering};

static OUT: AtomicIsize = AtomicIsize::new(0);

struct Counting;
unsafe impl Allocator for Counting {
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        OUT.fetch_add(1, Ordering::SeqCst);
        NonNull::new(unsafe { alloc_zeroed(layout) }).ok_or(AllocError)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        OUT.fetch_sub(1, Ordering::SeqCst);
        unsafe { dealloc(ptr.as_ptr(), layout) }
    }
}

fn main() {
    let map = BBTreeMap::<u64, u64, bbtree::Auto, Counting>::new_in(Counting);
    for k in 0..55 {
        map.insert(k, k).unwrap();
    }
    let caught = catch_unwind(AssertUnwindSafe(|| {
        map.get_or_insert(55, 55, |_, _| panic!("closure panics"))
    }));
    println!("caught: {}", caught.is_err());
    drop(map);
    println!("blocks still allocated after drop: {}", OUT.load(Ordering::SeqCst));
}
