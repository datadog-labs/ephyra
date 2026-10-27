Run: `./run.sh <path-to-bbtree-checkout>` (needs cargo, edition 2024).
- main.rs, debug build: 55 `u64 -> u64` entries fill the root leaf. Then `get_or_insert` of a new key runs a closure that panics, inside `catch_unwind`. On trunk the process aborts (exit 134).
- leak.rs, release build: the same steps with a counting `bbtree::Allocator`. It prints how many blocks are still allocated after the map is dropped (trunk: 2, expected 0).
Exit 1 = bug shows, 0 = it does not, 2 = build failed.
On f0a5d36: exit 1. With work/clusters/crash-abort-syncvacant/fix.diff applied: exit 0.
