//! Minimal background-task plumbing.
//!
//! Network calls must not block the UI thread, but Leetr deliberately avoids
//! an async runtime. Instead, jobs run on plain `std::thread`s and post their
//! results back through a channel that the UI drains once per frame.

use std::sync::mpsc::{self, Receiver, Sender};

/// A channel of results produced by background jobs.
pub struct Bg<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
}

impl<T> Bg<T> {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    /// Results posted since the last call, without blocking.
    pub fn drain(&self) -> Vec<T> {
        let mut collected = Vec::new();
        while let Ok(value) = self.rx.try_recv() {
            collected.push(value);
        }
        collected
    }

    /// True while no result is waiting (used to decide whether to repaint).
    pub fn is_empty(&self) -> bool {
        self.rx.try_recv().is_err()
    }
}

impl<T> Default for Bg<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> Bg<T> {
    /// Run `job` on a fresh thread and post its return value to the channel.
    pub fn spawn<F>(&self, job: F)
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_is_empty_before_any_job_runs() {
        let bg: Bg<u32> = Bg::new();
        assert!(bg.drain().is_empty());
        assert!(bg.is_empty());
    }

    #[test]
    fn spawned_jobs_post_their_results() {
        let bg = Bg::new();
        for value in 0..4u32 {
            bg.spawn(move || value * 2);
        }

        let mut collected = Vec::new();
        for _ in 0..200 {
            collected.extend(bg.drain());
            if collected.len() == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        collected.sort_unstable();
        assert_eq!(collected, vec![0, 2, 4, 6]);
        assert!(bg.drain().is_empty());
    }

    #[test]
    fn panicking_jobs_do_not_poison_the_channel() {
        let bg = Bg::new();
        bg.spawn(|| panic!("job blew up"));
        bg.spawn(|| 7u32);

        let mut collected = Vec::new();
        for _ in 0..200 {
            collected.extend(bg.drain());
            if !collected.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(collected, vec![7]);
    }
}
