# BBTreeMap: dropping the map while a key's or value's Drop panics leaks every page on its free lists

Posted as https://github.com/XiangpengHao/bbtree/issues/29 on 2026-10-27. The text below is the issue as posted.

---

## Summary

When a key's or value's `Drop` panics while a `BBTreeMap` is dropped, the pages on the map's free lists are never given back to the allocator. These are the pages of nodes that removals emptied, kept for reuse. In the program below, 26 of the map's 27 pages are never freed.

`Tree::destroy` (`bbtree/src/tree/teardown.rs:22-26`) says what should happen:

> If a key's or value's `Drop` panics, the rest is still taken apart, in the same order, before the panic goes on, as in std's collections. Either way, every key and value is dropped once and every page released once.

and `LatchedTree` (`bbtree/src/tree/latched/mod.rs:85-86`) owns "the pages on its free lists", which its `destroy` takes apart "the pages on its free lists too".

`Tree::destroy` keeps that promise for the tree's own nodes: it finishes the walk, then lets the panic go on. But `LatchedTree::destroy` calls it first and only then frees the free lists, so the unwinding panic skips that call.

std's `BTreeMap` with the same steps frees all its memory: allocations go back to the process's baseline after the drop panics.

## Reproducer

```rust
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
```

```
cargo run --release
# expected (what a Drop that does not panic gives):
#   pages out after drop: 0
# bbtree 1c72bd2:
pages out before drop: 27
drop panicked: true
pages out after drop: 26
```

The same in a debug build. It fails the same way on every run (deterministic, one thread).

## Notes

- `LatchedTree::destroy` (`bbtree/src/tree/latched/mod.rs:367-378`) calls `Tree::of_root_page(root_page).destroy(alloc)` and then `self.free.deallocate_all(..)`. When `Tree::destroy` re-raises the `Drop` panic, `deallocate_all` never runs. Freeing the free lists from a drop guard, so that they are freed on unwind too, fixes it.
- Only dropping the map leaks. `clear` (which also calls `destroy`) leaves the free lists on the map, so later inserts reuse them and the final drop frees them. `into_iter` and `unsync::BBTreeMap` (which keeps no free lists) free everything.
- Every release from 0.1.1 to 0.1.6 has this `LatchedTree::destroy`.

Found by an automated property-based tester (ephyra); drafted with AI assistance.
<!-- owner: add in your own words whether you reproduced this yourself -->

