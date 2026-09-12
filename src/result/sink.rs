use super::{Batch, Summary, summary};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

pub struct Sink {
    files: AtomicU64,
    batch: Mutex<Batch>,
}

impl Sink {
    pub fn new() -> Self {
        Self {
            files: AtomicU64::new(0),
            batch: Mutex::new(Batch::default()),
        }
    }

    pub fn dump(&self, batch: &mut Batch) {
        let files = batch.files();
        if files == 0 {
            return;
        }

        self.batch.lock().unwrap().absorb(batch);
        self.files.fetch_add(files, Ordering::Relaxed);
    }

    pub fn files(&self) -> u64 {
        self.files.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Summary {
        let batch = self.batch.lock().unwrap();
        summary::from_batch(&batch)
    }
}
