# WalWriter::open reports Corrupted on a healthy WAL when another writer fills the last file during the scan

Posted as https://github.com/XiangpengHao/bbtree/issues/23 on 2026-10-26. The text below is the issue as posted.

---

H i@XiangpengHao, amazing project. I pointed a new testing tool I'm working to it overnight and got a hit on a small bug.

## Summary
 
`WalWriter::open` can fail with `WalError::Corrupted { file: 1, offset: 12 }` on a WAL that is not corrupted.
It happens when another writer appends to the last file `n` and then creates file `n + 1` while the open is scanning: after the open reads file `n`, but before it asks for the length of file `n + 1`.
The scan moves on to file `n + 1` without reading the bytes added to file `n`. The first record of file `n + 1` then has an LSN one or more past the one the scan expects, and the LSN check fails.

`design/bbtree-store/01-wal.md` treats another writer appending during an open as a normal case:

> If the claim fails, another writer appended: scan on from where the scan ended, and claim again.

and, under Verification, that "A scan picks up where it stopped" (proven, `WalTail::lemma_scan_in_two`), so that "an open that scans on after its claim fails reads what a fresh one would". Here the open fails during its first scan, and a fresh open of the same bytes succeeds. The open still reports the WAL as corrupted, which the doc reserves for bytes that fail their checks:

> A failed check is corruption, an error, never the WAL's end

## Reproducer

Storage in memory with `MAX_APPENDS = 2`. Writer A creates the WAL (file 0 holds its claim). Writer B opens. Between B's read of file 0 and B's `len(1)`, writer C opens (its claim fills file 0) and appends one removal (which creates file 1). The interleaving is fixed by counting storage calls, so the program fails the same way on every run. It uses only `bbtree-store` and std.

```rust
//! A healthy WAL that another writer appends to during `WalWriter::open`
//! makes the open fail with `Corrupted`.
use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use bbtree_store::wal::{AppendError, Record, WalStorage, WalWriter};

/// Files in memory, shared by every handle.
#[derive(Clone, Default)]
struct Mem {
    files: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    /// If set: before this handle's storage call number `n` (from 0),
    /// another writer opens the WAL and appends one record.
    rival_before_call: Option<(u32, Arc<AtomicU32>)>,
}

impl Mem {
    fn tick(&self) {
        if let Some((n, calls)) = &self.rival_before_call {
            if calls.fetch_add(1, Ordering::SeqCst) == *n {
                let other = Mem { files: self.files.clone(), rival_before_call: None };
                let mut rival = block_on(WalWriter::open(other, |_, _| {})).unwrap();
                block_on(rival.append(&[Record::Remove { key: b"b" }])).unwrap();
            }
        }
    }
}

impl WalStorage for Mem {
    const MAX_APPENDS: u32 = 2;

    async fn append(&self, file: u64, offset: u64, bytes: &[u8]) -> Result<(), AppendError> {
        self.tick();
        let mut files = self.files.lock().unwrap();
        match files.get_mut(&file) {
            None if offset == 0 => {
                files.insert(file, bytes.to_vec());
            }
            Some(f) if offset != 0 && f.len() as u64 == offset => f.extend_from_slice(bytes),
            _ => return Err(AppendError::OffsetMismatch),
        }
        Ok(())
    }

    async fn len(&self, file: u64) -> io::Result<Option<u64>> {
        self.tick();
        Ok(self.files.lock().unwrap().get(&file).map(|f| f.len() as u64))
    }

    async fn read(&self, file: u64, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.tick();
        let files = self.files.lock().unwrap();
        let f = files.get(&file).ok_or(io::ErrorKind::NotFound)?;
        let bytes = f.get(offset as usize..offset as usize + buf.len()).ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

/// The storage never waits.
fn block_on<F: Future>(f: F) -> F::Output {
    match pin!(f).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => unreachable!(),
    }
}

fn main() {
    let mem = Mem::default();
    // Writer A creates the WAL: file 0 holds its claim (1 of 2 appends).
    drop(block_on(WalWriter::open(mem.clone(), |_, _| {})).unwrap());

    // Writer B opens: len(0), read(0), len(1), ... Between its read of
    // file 0 and its len(1), writer C opens (its claim fills file 0) and
    // appends a removal (which creates file 1).
    let b = Mem { rival_before_call: Some((2, Arc::new(AtomicU32::new(0)))), ..mem.clone() };
    let opened = block_on(WalWriter::open(b, |_, _| {}));
    println!("open during another writer's appends: {:?}", opened.as_ref().map(|_| "Ok"));

    // The same bytes, opened again with no other writer.
    let fresh = block_on(WalWriter::open(mem.clone(), |_, _| {}));
    println!("fresh open of the same WAL:          {:?}", fresh.as_ref().map(|_| "Ok"));
}
```

```
cargo run
# expected (a fresh open of the same bytes succeeds):
#   open during another writer's appends: Ok("Ok")
#   fresh open of the same WAL:          Ok("Ok")
# bbtree f0a5d36:
#   open during another writer's appends: Err(Corrupted { file: 1, offset: 12 })
#   fresh open of the same WAL:          Ok("Ok")
```

## Notes

- `bbtree-store/src/wal/writer.rs:90-104`, `WalTail::scan`: after `scan_file` reads file `n` up to the length it saw, it calls `storage.len(self.file + 1)` (line 98). If file `n + 1` exists, it moves to it (line 100) and keeps the next LSN from what it read. Bytes appended to file `n` after it was read are never scanned.
- The proven property covers a scan picking up within the bytes it is given; the gap is in the step that moves from one file to the next, in `writer.rs`, outside the proof.
- A writer creates file `n + 1` only once file `n` is full. So once `n + 1` exists, file `n`'s length is final. Re-reading file `n`'s length at that point, and scanning the rest of it before moving on, closes the gap.
- This needs no faults: only two writers on the same storage, which the claim protocol is built for (a failed-over writer still alive). The real-world window is between one `read` and the next `len` of an open.
- The WAL is not in a release yet (v0.1.5 on crates.io predates #17).

Found by an automated property-based tester (ephyra); drafted with AI assistance.
