Run: `./run.sh <path-to-bbtree-checkout>` (needs cargo; edition 2024, std `Waker::noop`).
Prints both opens' results. Exit 1 = open fails with Corrupted (bug), 0 = it succeeds.
On f0a5d36: exit 1. With patches/001 applied: exit 0.
