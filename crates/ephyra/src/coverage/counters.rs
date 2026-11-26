// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Counter snapshots and function records read from the profile runtime.

use std::{
    collections::{
        BTreeMap,
        HashMap,
        HashSet,
    },
    ops::Range,
    sync::{
        Arc,
        Condvar,
        Mutex,
        OnceLock,
    },
};

use super::{
    mapping::Mapping,
    names,
};

/// Whether this binary was built with `-C instrument-coverage` and the
/// profile runtime is linked in.
pub fn available() -> bool {
    cfg!(ephyra_instrumented)
}

static HELD: Mutex<bool> = Mutex::new(false);
static FREED: Condvar = Condvar::new();

/// Holds the process-wide coverage lock; dropping it releases the lock.
/// Unlike a `MutexGuard` it may move between threads, so a [`Run`](crate::Run)
/// can own one.
#[derive(Debug)]
pub(crate) struct Lease(());

impl Drop for Lease {
    fn drop(&mut self) {
        let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *held = false;
        FREED.notify_one();
    }
}

/// The process-wide lock coverage-using phases hold so that ephyra's own
/// runs do not pollute each other's deltas. Other test threads are not
/// stopped by it; run with `RUST_TEST_THREADS=1` for exact attribution.
pub(crate) fn lock() -> Lease {
    let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while *held {
        held = FREED.wait(held).unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    *held = true;
    Lease(())
}

/// The counter section (or a window of it) at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    offset: usize,
    counters: Vec<u64>,
}

impl Snapshot {
    /// Number of counters in the binary.
    pub fn len(&self) -> usize {
        self.counters.len()
    }

    /// Whether the binary has no counters (always true when uninstrumented).
    pub fn is_empty(&self) -> bool {
        self.counters.is_empty()
    }

    /// The counter values.
    pub fn counters(&self) -> &[u64] {
        &self.counters
    }

    /// Counters that increased between `self` and `later` (both must cover
    /// the same window). Indices are absolute counter indices.
    pub fn delta(&self, later: &Snapshot) -> Delta {
        let hits = self
            .counters
            .iter()
            .zip(&later.counters)
            .enumerate()
            .filter(|(_, (before, after))| after > before)
            .map(|(index, (before, after))| (self.offset + index, after - before))
            .collect();
        Delta { hits }
    }
}

/// Counters that increased between two snapshots, as `(index, increment)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    hits: Vec<(usize, u64)>,
}

impl Delta {
    /// The counters hit, with how many times each was incremented.
    pub fn hits(&self) -> &[(usize, u64)] {
        &self.hits
    }

    /// Whether any counter was hit.
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// Whether the counter at `index` was hit.
    pub fn touches(&self, index: usize) -> bool {
        self.hits.binary_search_by_key(&index, |(i, _)| *i).is_ok()
    }

    /// Restrict to counters inside `range`.
    pub fn within(&self, range: &Range<usize>) -> Delta {
        Delta { hits: self.hits.iter().copied().filter(|(i, _)| range.contains(i)).collect() }
    }

    /// Set `values` to the increments of the counters in `range`, one slot
    /// per counter; `false`, leaving `values` alone, when none was hit.
    pub(crate) fn fill(&self, range: &Range<usize>, values: &mut Vec<u64>) -> bool {
        let first = self.hits.partition_point(|(i, _)| *i < range.start);
        if first >= self.hits.len() || self.hits[first].0 >= range.end {
            return false;
        }
        values.clear();
        values.resize(range.len(), 0);
        for &(i, count) in &self.hits[first..] {
            if i >= range.end {
                break;
            }
            values[i - range.start] = count;
        }
        true
    }
}

/// One instrumented function: its demangled name and the counters that
/// belong to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// Demangled name, e.g. `my_crate::module::function`.
    pub name: String,
    /// Hash of the profile name, the key shared with the coverage mapping.
    pub(crate) name_ref: u64,
    /// Structural hash of the function, the other half of that key.
    pub(crate) func_hash: u64,
    /// Indices into a [`Snapshot`] of this function's counters.
    pub counters: Range<usize>,
}

/// Keep the profile runtime from writing `default_<hash>_<pid>.profraw`
/// into the working directory when this process exits, unless someone
/// asked for a file by setting `LLVM_PROFILE_FILE` (as `cargo llvm-cov`
/// does). Opt-in: ephyra never calls it for you, because a profile
/// someone else is waiting for would silently vanish. Call it from any test
/// when the suite is instrumented only for ephyra's sake; the
/// `[env]` setting described in the [module docs](crate::coverage) is the
/// broader fix. A no-op when uninstrumented, and
/// [`Config::coverage.profraw_dir`](crate::Config) is unaffected.
pub fn suppress_default_dump() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        let asked = std::env::var_os("LLVM_PROFILE_FILE").is_some_and(|v| !v.is_empty());
        if !asked {
            runtime::set_dumped();
        }
    });
}

/// Every instrumented function with code in the binary, from the profile data
/// records and the names section. Built once per process.
#[derive(Debug)]
pub struct Functions {
    functions: Vec<Function>,
    /// The mangled names of the binary, by hash.
    names: HashMap<u64, String>,
    /// What [`absent_in`](Self::absent_in) found, by crate.
    absent: Mutex<HashMap<String, Arc<[Function]>>>,
}

impl Functions {
    /// The instrumented functions, or an empty list when uninstrumented.
    pub fn get() -> &'static Functions {
        static FUNCTIONS: OnceLock<Functions> = OnceLock::new();
        FUNCTIONS.get_or_init(|| {
            let (functions, names) = runtime::functions();
            Functions { functions, names, absent: Mutex::default() }
        })
    }

    /// All functions.
    pub fn all(&self) -> &[Function] {
        &self.functions
    }

    /// The functions defined in the crate of the normalized `filter` (in any
    /// crate when it is empty) that have no code in the binary, and so no data
    /// record and no counters: nothing calls them, but the coverage mapping
    /// lists them. Found once per crate and process; empty without a mapping.
    pub(crate) fn absent_in(&self, filter: &str) -> Arc<[Function]> {
        let krate = filter.split("::").next().unwrap_or_default();
        let mut absent = self.absent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        absent.entry(krate.to_string()).or_insert_with(|| self.find_absent(krate).into()).clone()
    }

    /// Instantiations of one definition count once, under the shortest name,
    /// and not at all when any of them has a record.
    fn find_absent(&self, krate: &str) -> Vec<Function> {
        let Some(mapping) = Mapping::get() else { return Vec::new() };
        // The way v0 mangling writes the crate, which most names do not mention.
        let separated = krate.starts_with(|c: char| c == '_' || c.is_ascii_digit());
        let separator = if separated { "_" } else { "" };
        let mention = format!("{}{separator}{krate}", krate.len());
        let definition = |name_ref: u64| {
            let name = self.names.get(&name_ref)?;
            if krate.is_empty() {
                return names::definition(name);
            }
            let definition = name.contains(&mention).then(|| names::definition(name))??;
            definition.strip_prefix(krate)?.starts_with("::").then_some(definition)
        };
        let recorded: HashSet<u64> = self.functions.iter().map(|f| f.name_ref).collect();
        let definitions: HashSet<String> =
            recorded.iter().filter_map(|&name_ref| definition(name_ref)).collect();
        let mut absent: BTreeMap<String, Function> = BTreeMap::new();
        for (name_ref, func_hash) in mapping.keys().filter(|(r, _)| !recorded.contains(r)) {
            let Some(definition) = definition(name_ref).filter(|d| !definitions.contains(d)) else {
                continue;
            };
            let name = names::pretty(&self.names[&name_ref]);
            let function = Function { name, name_ref, func_hash, counters: 0..0 };
            let shortest = absent.entry(definition).or_insert_with(|| function.clone());
            if (function.name.len(), &function.name) < (shortest.name.len(), &shortest.name) {
                *shortest = function;
            }
        }
        absent.into_values().collect()
    }

    /// Functions whose demangled name starts with `prefix` (a crate name,
    /// or `crate::module`).
    pub fn in_crate<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a Function> + 'a {
        self.functions.iter().filter(move |f| {
            f.name.starts_with(prefix) && f.name[prefix.len()..].starts_with("::")
                || f.name.trim_start_matches('<').starts_with(prefix)
        })
    }

    /// Functions touched by `delta`.
    pub fn reached<'a>(&'a self, delta: &'a Delta) -> impl Iterator<Item = &'a Function> + 'a {
        self.functions.iter().filter(|f| delta.hits.iter().any(|(i, _)| f.counters.contains(i)))
    }
}

/// A snapshot of the whole counter section, or `None` when uninstrumented.
pub fn snapshot() -> Option<Snapshot> {
    runtime::counters(None).map(|counters| Snapshot { offset: 0, counters })
}

/// A snapshot of the counters in `range` only; cheaper than the whole
/// section when only one crate matters.
pub(crate) fn snapshot_range(range: &Range<usize>) -> Option<Snapshot> {
    runtime::counters(Some(range.clone()))
        .map(|counters| Snapshot { offset: range.start, counters })
}

/// A profraw image of the process's counters, for `llvm-profdata` and
/// `llvm-cov`. `None` when uninstrumented.
pub fn profraw_image() -> Option<Vec<u8>> {
    runtime::profraw()
}

/// Where the counters start, given where the profile runtime says their
/// section does. On Windows that is the byte after a one-byte marker, and the
/// linker puts the first counter at the next multiple of 8.
#[cfg(any(test, ephyra_instrumented))]
fn first_counter(begin: *const u8) -> *const u8 {
    begin.wrapping_add(begin.align_offset(8))
}

/// The only unsafe code in the crate: FFI into the LLVM profile runtime and
/// reads of its static sections, whose layout is fixed by compiler-rt.
#[cfg(ephyra_instrumented)]
#[allow(unsafe_code)]
mod runtime {
    /// Mark the profile as already written so the exit handler skips it.
    pub fn set_dumped() {
        unsafe { __llvm_profile_set_dumped() }
    }

    use std::{
        collections::HashMap,
        sync::atomic::{
            AtomicU64,
            Ordering,
        },
    };

    use super::{
        super::names,
        Function,
        first_counter,
    };

    unsafe extern "C" {
        fn __llvm_profile_set_dumped();
        fn __llvm_profile_begin_counters() -> *mut u8;
        fn __llvm_profile_end_counters() -> *mut u8;
        fn __llvm_profile_begin_data() -> *const u8;
        fn __llvm_profile_end_data() -> *const u8;
        fn __llvm_profile_begin_names() -> *const u8;
        fn __llvm_profile_end_names() -> *const u8;
        fn __llvm_profile_counter_entry_size() -> usize;
        fn __llvm_profile_get_size_for_buffer() -> u64;
        fn __llvm_profile_write_buffer(buffer: *mut u8) -> i32;
    }

    /// Layouts of a `__llvm_profile_data` record: its size and the offset of
    /// `NumCounters`. `NameRef`, `FuncHash` and the relative `CounterPtr`
    /// lead every layout. LLVM 23 (rustc 1.99) added an 8-byte field before
    /// `NumCounters`; LLVM 20 to 22 records are 64 bytes.
    const LAYOUTS: [(usize, usize); 2] = [(72, 56), (64, 48)];

    /// The layout under which the data section divides into whole records
    /// whose `NameRef`s are names in the names section, judged on the first
    /// records; `None` when no layout fits.
    fn layout(records: &[u8], by_ref: &HashMap<u64, &str>) -> Option<(usize, usize)> {
        LAYOUTS
            .iter()
            .copied()
            .filter(|(size, _)| !records.is_empty() && records.len() % size == 0)
            .map(|(size, num_counters_at)| {
                let resolved = records
                    .chunks_exact(size)
                    .take(64)
                    .filter(|r| {
                        by_ref
                            .contains_key(&u64::from_le_bytes(r[0..8].try_into().expect("8 bytes")))
                    })
                    .count();
                (resolved, size, num_counters_at)
            })
            .filter(|(resolved, ..)| *resolved > 0)
            .max_by_key(|(resolved, ..)| *resolved)
            .map(|(_, size, num_counters_at)| (size, num_counters_at))
    }

    pub(super) fn counters(range: Option<std::ops::Range<usize>>) -> Option<Vec<u64>> {
        // SAFETY: the profile runtime owns a contiguous, process-lifetime
        // counter section; the compiler increments its entries atomically.
        unsafe {
            if __llvm_profile_counter_entry_size() != 8 {
                return None;
            }
            let begin = first_counter(__llvm_profile_begin_counters());
            let end = __llvm_profile_end_counters();
            let count = end.offset_from(begin).max(0) as usize / 8;
            let slice = std::slice::from_raw_parts(begin as *const AtomicU64, count);
            let range = range.map_or(0..count, |r| r.start.min(count)..r.end.min(count));
            Some(slice[range].iter().map(|c| c.load(Ordering::Relaxed)).collect())
        }
    }

    pub(super) fn profraw() -> Option<Vec<u8>> {
        // SAFETY: write_buffer fills exactly get_size_for_buffer bytes.
        unsafe {
            let size = __llvm_profile_get_size_for_buffer() as usize;
            let mut buffer = vec![0u8; size];
            if __llvm_profile_write_buffer(buffer.as_mut_ptr()) != 0 {
                return None;
            }
            Some(buffer)
        }
    }

    pub(super) fn functions() -> (Vec<Function>, HashMap<u64, String>) {
        // SAFETY: the data and names sections are static, contiguous, and
        // laid out as documented in compiler-rt's InstrProfData.inc.
        let (records, names, counters_base) = unsafe {
            let begin = __llvm_profile_begin_data();
            let end = __llvm_profile_end_data();
            let len = end.offset_from(begin).max(0) as usize;
            let records = std::slice::from_raw_parts(begin, len);
            let nb = __llvm_profile_begin_names();
            let ne = __llvm_profile_end_names();
            let names = std::slice::from_raw_parts(nb, ne.offset_from(nb).max(0) as usize);
            (records, names, first_counter(__llvm_profile_begin_counters()) as usize)
        };
        let names = names::parse(names);
        let by_ref = names::by_ref(&names);
        let Some((record_size, num_counters_at)) = layout(records, &by_ref) else {
            return Default::default();
        };
        let mut functions = Vec::with_capacity(records.len() / record_size);
        for (index, record) in records.chunks_exact(record_size).enumerate() {
            let name_ref = u64::from_le_bytes(record[0..8].try_into().expect("8 bytes"));
            let func_hash = u64::from_le_bytes(record[8..16].try_into().expect("8 bytes"));
            let counter_ptr = i64::from_le_bytes(record[16..24].try_into().expect("8 bytes"));
            let num_counters = u32::from_le_bytes(
                record[num_counters_at..num_counters_at + 4].try_into().expect("4 bytes"),
            ) as usize;
            let record_addr = unsafe { __llvm_profile_begin_data() as usize } + index * record_size;
            let counter_addr = (record_addr as i64 + counter_ptr) as usize;
            if counter_addr < counters_base || num_counters == 0 {
                continue;
            }
            let start = (counter_addr - counters_base) / 8;
            let name = by_ref
                .get(&name_ref)
                .map_or_else(|| format!("<unknown {name_ref:#x}>"), |n| names::pretty(n));
            functions.push(Function {
                name,
                name_ref,
                func_hash,
                counters: start..start + num_counters,
            });
        }
        functions.sort_by_key(|f| f.counters.start);
        let names = by_ref.into_iter().map(|(hash, name)| (hash, name.to_string())).collect();
        (functions, names)
    }
}

#[cfg(not(ephyra_instrumented))]
mod runtime {
    pub fn set_dumped() {}

    use std::collections::HashMap;

    use super::Function;

    pub(super) fn counters(_range: Option<std::ops::Range<usize>>) -> Option<Vec<u64>> {
        None
    }

    pub(super) fn functions() -> (Vec<Function>, HashMap<u64, String>) {
        Default::default()
    }

    pub(super) fn profraw() -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_and_within() {
        let before = Snapshot { offset: 0, counters: vec![1, 5, 0, 7] };
        let after = Snapshot { offset: 0, counters: vec![1, 9, 3, 7] };
        let delta = before.delta(&after);
        assert_eq!(delta.hits(), &[(1, 4), (2, 3)]);
        assert!(delta.touches(2) && !delta.touches(3));
        assert_eq!(delta.within(&(2..4)).hits(), &[(2, 3)]);
        assert!(
            Snapshot { offset: 0, counters: vec![] }
                .delta(&Snapshot { offset: 0, counters: vec![] })
                .is_empty()
        );
        let windowed = Snapshot { offset: 10, counters: vec![0, 1] }
            .delta(&Snapshot { offset: 10, counters: vec![0, 2] });
        assert_eq!(windowed.hits(), &[(11, 1)]);
        let mut values = vec![9];
        assert!(!delta.fill(&(3..5), &mut values) && values == [9]);
        assert!(delta.fill(&(0..3), &mut values) && values == [0, 4, 3]);
    }

    #[test]
    fn the_first_counter_is_the_next_multiple_of_8() {
        let section = [0u64; 2];
        let start = section.as_ptr().cast::<u8>();
        assert_eq!(first_counter(start), start);
        for skew in 1..=8 {
            assert_eq!(first_counter(start.wrapping_add(skew)), start.wrapping_add(8));
        }
    }

    /// A counter reads as the number of times its code ran, also when that
    /// does not fit in its low byte.
    #[cfg(ephyra_instrumented)]
    #[test]
    fn counts_are_exact() {
        #[inline(never)]
        fn counted(x: u32) -> u32 {
            x.wrapping_add(1)
        }
        let all = Functions::get().all();
        let function = all.iter().find(|f| f.name.ends_with("counts_are_exact::counted")).unwrap();
        for n in [5, 256] {
            let before = snapshot_range(&function.counters).unwrap();
            for x in 0..n {
                std::hint::black_box(counted(std::hint::black_box(x)));
            }
            let delta = before.delta(&snapshot_range(&function.counters).unwrap());
            assert_eq!(delta.hits(), &[(function.counters.start, u64::from(n))]);
        }
    }

    #[test]
    fn availability_matches_the_build() {
        assert_eq!(available(), snapshot().is_some());
        if available() {
            let functions = Functions::get();
            assert!(!functions.all().is_empty());
            assert!(
                functions.all().iter().any(|f| f.name.contains("ephyra")),
                "{:?}",
                &functions.all()[..5.min(functions.all().len())]
            );
            assert!(!functions.absent_in("ephyra").is_empty());
            // `rand_chacha` defines the impls of `rand_core` traits for its types.
            let absent = functions.absent_in("rand_core");
            assert!(!absent.iter().any(|f| f.name.contains("rand_chacha")), "{absent:?}");
            // Running code increments its counters.
            let before = snapshot().unwrap();
            let _ = std::hint::black_box(names::md5(b"delta"));
            let after = snapshot().unwrap();
            let delta = before.delta(&after);
            assert!(
                functions.reached(&delta).any(|f| f.name.contains("md5")),
                "{:?}",
                functions.reached(&delta).map(|f| &f.name).collect::<Vec<_>>()
            );
        }
    }
}
