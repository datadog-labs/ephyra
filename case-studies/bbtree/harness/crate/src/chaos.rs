// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Schedule perturbation for the `bbz-chaos` build (README, "bbz-chaos").
//!
//! bbtree marks where a latched tree's threads may interleave
//! (`bbtree/src/tree/interleaving.rs`): before each access to an atomic that
//! threads share (latch words, versions, the root, page counts), and in each
//! round of waiting for a latch. Under Shuttle those points switch threads;
//! in a normal build they are nothing. build.sh's chaos build copies the
//! sources and makes `Interleaving::switch_point` call
//! [`bbz_chaos_switch_point`] and `Interleaving::wait` call
//! [`bbz_chaos_wait`], so a thread stops for a while just where another
//! thread's view of a node can go stale: between an optimistic read and its
//! validation, while holding a latch, between a split's halves. Nothing else
//! changes: what the tree does at each point is what a preempted thread
//! would do, only more often.
//!
//! Each thread draws a mode every few hundred to few thousand points: quiet,
//! short spins, long spins, yields, or rare stalls (a thread that lost its
//! CPU while holding a latch). In the plain and ASan builds nothing calls
//! these functions.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// The chaos build (`--cfg bbz_chaos`): workers run only the threaded
/// properties, the ones the perturbation can change.
pub const ACTIVE: bool = cfg!(bbz_chaos);

static THREADS: AtomicU64 = AtomicU64::new(0);

/// `BBZ_CHAOS_SHIFT` (for tuning; default 3): each mode's perturbations are
/// 2^(shift + 0..=4) times rarer than the table in [`bbz_chaos_switch_point`]
/// says. Measured on synclin (3 workers, 60 s, plain build 2940 cases): shift
/// 2: 429 cases, 4: 898, 6: 1703.
fn base_shift() -> u32 {
    static SHIFT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SHIFT.get_or_init(|| std::env::var("BBZ_CHAOS_SHIFT").ok().and_then(|s| s.parse().ok()).unwrap_or(3).min(20))
}

thread_local! {
    /// xorshift state (0 = not seeded yet), mode, its shift, points left in
    /// the mode.
    static STATE: Cell<(u64, u8, u32, u32)> = const { Cell::new((0, 0, 0, 0)) };
}

fn next(rng: &mut u64) -> u64 {
    if *rng == 0 {
        let n = THREADS.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        *rng = (n.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ t) | 1;
    }
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn spin(n: u64) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

/// Before a shared atomic's access in a latched tree.
#[unsafe(no_mangle)]
pub extern "Rust" fn bbz_chaos_switch_point() {
    let _ = STATE.try_with(|s| {
        let (mut rng, mut mode, mut shift, mut left) = s.get();
        let r = next(&mut rng);
        if left == 0 {
            // quiet 30%, light 25%, heavy 20%, yields 15%, stalls 10%
            mode = match (r >> 8) % 20 {
                0..6 => 0,
                6..11 => 1,
                11..15 => 2,
                15..18 => 3,
                _ => 4,
            };
            shift = base_shift() + ((r >> 40) % 5) as u32;
            left = 256 + ((r >> 24) % 4096) as u32;
        }
        left -= 1;
        s.set((rng, mode, shift, left));
        // each perturbation is 2^shift times rarer than at shift 0
        let roll = r & 0xFFFF_FFFF;
        let one_in = |n: u64| roll % (n << shift);
        match mode {
            1 if one_in(16) == 0 => spin((r >> 32) % 512),
            2 if one_in(4) == 0 => spin((r >> 32) % 8192),
            3 if one_in(32) == 0 => std::thread::yield_now(),
            4 if one_in(2048) == 0 => spin(4096 + (r >> 32) % 32768),
            4 if one_in(64) == 1 => std::thread::yield_now(),
            _ => {}
        }
    });
}

/// One round of waiting for a latch, `spins` spin hints long (bbtree's
/// `Backoff`): the spins, and now and then a yield, so a holder stalled by
/// [`bbz_chaos_switch_point`] gets a CPU back when threads outnumber CPUs.
#[unsafe(no_mangle)]
pub extern "Rust" fn bbz_chaos_wait(spins: u32) {
    spin(u64::from(spins));
    let _ = STATE.try_with(|s| {
        let (mut rng, mode, shift, left) = s.get();
        let r = next(&mut rng);
        s.set((rng, mode, shift, left));
        if r % 8 == 0 {
            std::thread::yield_now();
        }
    });
}
