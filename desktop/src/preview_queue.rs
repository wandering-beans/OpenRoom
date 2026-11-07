//! Bounded decoder admission. Interactive requests overtake queued background work.
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Condvar, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Pending {
    ticket: u64,
    waiting: VecDeque<(u64, i64, u8)>,
    active: HashMap<i64, u8>,
    completed: bool,
}
pub struct Queue {
    capacity: usize,
    initial_capacity: usize,
    pending: Mutex<Pending>,
    changed: Condvar,
}
pub struct Job<'a> {
    queue: &'a Queue,
    id: i64,
}
#[cfg(target_os = "windows")]
fn default_capacity() -> usize {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    if std::thread::available_parallelism().map_or(1, usize::from) < 16 {
        return 2;
    }
    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // Four RAW workers peaked near 4 GiB in the Windows benchmark. Keep room for the editor and OS.
    if unsafe { GlobalMemoryStatusEx(&mut memory) }.is_ok()
        && memory.ullTotalPhys >= 30 * 1024u64.pow(3)
        && memory.ullAvailPhys >= 8 * 1024u64.pow(3)
    {
        4
    } else {
        2
    }
}

#[cfg(not(target_os = "windows"))]
fn default_capacity() -> usize {
    2
}

impl Default for Queue {
    fn default() -> Self {
        match std::env::var("AE_BENCH_IMPORT_WORKERS").as_deref() {
            Ok("2") => Self::new(2, 2),
            Ok("4") => Self::new(4, 4),
            _ => Self::new(default_capacity(), 2),
        }
    }
}

impl Queue {
    fn new(capacity: usize, initial_capacity: usize) -> Self {
        Self {
            capacity,
            initial_capacity,
            pending: Mutex::default(),
            changed: Condvar::new(),
        }
    }
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn enter(
        &self,
        id: i64,
        priority: u8,
        cancelled: &dyn Fn() -> bool,
        foreground_busy: &dyn Fn() -> bool,
    ) -> Result<Job<'_>> {
        let mut pending = self.pending.lock().unwrap();
        let ticket = pending.ticket;
        pending.ticket += 1;
        pending.waiting.push_back((ticket, id, priority));
        loop {
            if cancelled() {
                pending.waiting.retain(|job| job.0 != ticket);
                self.changed.notify_all();
                anyhow::bail!("プレビュー生成を中止しました。");
            }
            let next = pending
                .waiting
                .iter()
                .filter(|job| !pending.active.contains_key(&job.1))
                .max_by_key(|job| (job.2, std::cmp::Reverse(job.0)))
                .map(|job| job.0);
            let limit = if pending.completed {
                self.capacity
            } else {
                self.initial_capacity
            };
            if pending.active.len() < limit
                && next == Some(ticket)
                && (priority == 2 || !foreground_busy())
            {
                pending.waiting.retain(|job| job.0 != ticket);
                ensure!(pending.active.insert(id, priority).is_none());
                return Ok(Job { queue: self, id });
            }
            pending = self
                .changed
                .wait_timeout(pending, Duration::from_millis(10))
                .unwrap()
                .0;
        }
    }
    pub fn wants_large(&self, id: i64) -> bool {
        let pending = self.pending.lock().unwrap();
        pending.active.get(&id) == Some(&2)
            || pending.waiting.iter().any(|job| job.1 == id && job.2 == 2)
    }
}
impl Drop for Job<'_> {
    fn drop(&mut self) {
        let mut pending = self.queue.pending.lock().unwrap();
        pending.active.remove(&self.id);
        pending.completed = true;
        drop(pending);
        self.queue.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    #[test]
    fn larger_queue_starts_small_until_the_first_job_finishes() {
        let queue = Queue::new(4, 2);
        let first = queue.enter(1, 0, &|| false, &|| false).unwrap();
        let _second = queue.enter(2, 0, &|| false, &|| false).unwrap();
        std::thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            let queue_ref = &queue;
            scope.spawn(move || {
                let job = queue_ref.enter(3, 0, &|| false, &|| false).unwrap();
                tx.send(job).unwrap();
            });
            assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
            drop(first);
            let _third = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let _fourth = queue.enter(4, 0, &|| false, &|| false).unwrap();
            let _fifth = queue.enter(5, 0, &|| false, &|| false).unwrap();
        });
    }

    #[test]
    fn selected_photo_overtakes_background_and_duplicate_waits() {
        let queue = Arc::new(Queue::new(2, 2));
        let a = queue.enter(1, 0, &|| false, &|| false).unwrap();
        let b = queue.enter(2, 0, &|| false, &|| false).unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let mut releases = Vec::new();
            for (id, priority) in [(3, 0), (4, 1), (5, 2), (1, 2)] {
                let queue = queue.clone();
                let tx = tx.clone();
                let (release, wait) = mpsc::channel();
                releases.push(release);
                scope.spawn(move || {
                    let _job = queue.enter(id, priority, &|| false, &|| false).unwrap();
                    tx.send(id).unwrap();
                    wait.recv().unwrap();
                });
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while queue.pending.lock().unwrap().waiting.len() != 4 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            drop(b);
            assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 5);
            assert_eq!(queue.pending.lock().unwrap().active.len(), 2);
            releases[2].send(()).unwrap();
            assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 4);
            releases[1].send(()).unwrap();
            assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
            releases[0].send(()).unwrap();
            drop(a);
            assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
            releases[3].send(()).unwrap();
        });
        assert!(queue.pending.lock().unwrap().active.is_empty());
        let cancelled = AtomicBool::new(true);
        assert!(
            queue
                .enter(9, 2, &|| cancelled.load(Ordering::Relaxed), &|| false)
                .is_err()
        );
        assert!(queue.pending.lock().unwrap().waiting.is_empty());
    }
}
