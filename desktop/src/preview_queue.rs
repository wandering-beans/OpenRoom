//! Two decoders at most; interactive requests overtake queued background work.
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
}
#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    changed: Condvar,
}
pub struct Job<'a> {
    queue: &'a Queue,
    id: i64,
}
impl Queue {
    pub fn enter(&self, id: i64, priority: u8, cancelled: &dyn Fn() -> bool) -> Result<Job<'_>> {
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
            if pending.active.len() < 2 && next == Some(ticket) {
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
        self.queue.pending.lock().unwrap().active.remove(&self.id);
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
    fn selected_photo_overtakes_background_and_duplicate_waits() {
        let queue = Arc::new(Queue::default());
        let a = queue.enter(1, 0, &|| false).unwrap();
        let b = queue.enter(2, 0, &|| false).unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let mut releases = Vec::new();
            for (id, priority) in [(3, 0), (4, 1), (5, 2), (1, 2)] {
                let queue = queue.clone();
                let tx = tx.clone();
                let (release, wait) = mpsc::channel();
                releases.push(release);
                scope.spawn(move || {
                    let _job = queue.enter(id, priority, &|| false).unwrap();
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
                .enter(9, 2, &|| cancelled.load(Ordering::Relaxed))
                .is_err()
        );
        assert!(queue.pending.lock().unwrap().waiting.is_empty());
    }
}
