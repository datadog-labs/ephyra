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
