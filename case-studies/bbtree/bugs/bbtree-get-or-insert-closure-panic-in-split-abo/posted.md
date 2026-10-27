# BBTreeMap::get_or_insert: a panicking closure during a leaf split aborts the process (debug) or leaks the reserved pages (release)

Posted as https://github.com/XiangpengHao/bbtree/issues/25 on 2026-10-26. The text below is the issue as posted.

---

Hi @XiangpengHao , sharing another small hit.

## Summary

`BBTreeMap::get_or_insert` runs its closure on a new key "just before it goes in". If that key's leaf is full, the split reserves its pages first, then runs the closure. If the closure panics, the reserved pages are never given back.

With debug assertions on, `NodePages`'s `Drop` then fails its assertion while the panic unwinds, and Rust aborts the process. `catch_unwind` cannot catch it. In a release build the process goes on, but the reserved pages are never deallocated, not even when the map is dropped.

`design/09-concurrency.md` says a panic in a closure unwinds:

> Whatever is simplest. The latch guard unlatches on unwind; there is no poisoning; a value that a panicking `update` closure left half-changed stays as it is.

D48 in `design/03-decisions.md` lets the map do "whatever is simplest" after a panic in user code ("a panic need not leave a valid map behind"), on one condition:

> User code runs between structural changes, not inside one, so unwinding never leaves what unchecked code trusts (lengths, node minimums) half done (D26, D37). A change that must call it inside one goes behind an abort-on-unwind guard, as a key's hash tag (D36), and the window an internal node reads again from its key (D43), do.

Here the closure runs inside a structural change, after the split has reserved its pages, and without such a guard. The abort comes from a debug assertion in a `Drop` impl, and a release build leaks the reserved pages.

std's `BTreeMap` with the same steps (`entry(55).or_insert_with(|| panic!())` inside `catch_unwind`) unwinds, and the map still holds 55 entries.

## Reproducer

Debug build (`cargo run`):

```rust
use bbtree::BBTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn main() {
    let map = BBTreeMap::<u64, u64>::new();
    for k in 0..55 {
        map.insert(k, k).unwrap(); // 55 entries fill the root leaf
    }
    // A new key: the insert splits the leaf, and the closure panics.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        map.get_or_insert(55, 55, |_, _| panic!("closure panics"))
    }));
    println!("caught: {}", caught.is_err());
}
```

```
cargo run
# std BTreeMap (entry().or_insert_with): prints "caught: true", exit 0
# bbtree f0a5d36:
thread 'main' panicked at src/main.rs:11:42:
closure panics

thread 'main' panicked at bbtree/src/tree/latched/pages.rs:339:9:
assertion `left == right` failed: an operation allocates every page it reserved
  left: 2
 right: 0

thread 'main' panicked at library/core/src/panicking.rs:233:5:
panic in a destructor during cleanup
thread caused non-unwinding panic. aborting.
# exit 134 (SIGABRT)
```

With 54 entries (no split) it unwinds normally. Release build, the same steps with a counting allocator:

```rust
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
```

```
cargo run --release
# expected: blocks still allocated after drop: 0 (what a closure that does not panic gives)
# bbtree f0a5d36:
caught: true
blocks still allocated after drop: 2
```

Both fail the same way on every run (deterministic, one thread).

## Notes

`LatchedTree::split` in `bbtree/src/tree/latched/insert.rs` (around lines 388-398) calls `pages.reserve(..)` and then `policy.vacant(..)`. For `get_or_insert` (`ReadOrInsert`), `vacant` runs the user's closure. The refusal path (`Err(layout)`) calls `pages.unreserve()`, but a panic skips it. `NodePages::drop` (`bbtree/src/tree/latched/pages.rs:336-342`) then sees reserved pages left. `insert_into` and `append` reserve nothing before `vacant`, and `update_or_insert`'s `vacant` runs no user code, so the split in `get_or_insert` looks like the only public trigger. The `vacant` call after `reserve` came in with e185682 ("bbtree store"), so v0.1.5 has it too.

A fix with a test is available if useful.

Found by an automated property-based tester (ephyra); drafted with AI assistance.
<!-- owner: add in your own words whether you reproduced this yourself -->
