//! Runs the folder pickers' disk reads off the UI thread, on a bounded number of threads.
use std::sync::{Arc, Mutex, OnceLock};

/// Reads in flight at once, across every picker (only one picker dialog is open at a time).
const MAX_SCANS: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    running: usize,
    waiting: Option<Job>,
}

/// A read blocked on a hung NFS server or automount cannot be cancelled, so each click starting a thread would pile
/// them up. Instead at most `max` run; work arriving while all are busy waits in one slot, where newer work replaces
/// older (the user has moved on), and starts when a thread frees up.
#[doc(hidden)]
#[derive(Clone)]
pub struct Scanner {
    max: usize,
    state: Arc<Mutex<State>>,
}

impl Scanner {
    pub fn new(max: usize) -> Scanner {
        Scanner { max, state: Arc::default() }
    }

    /// The one the pickers share.
    pub fn shared() -> &'static Scanner {
        static SHARED: OnceLock<Scanner> = OnceLock::new();
        SHARED.get_or_init(|| Scanner::new(MAX_SCANS))
    }

    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        self.state.lock().expect("scanner poisoned").waiting = Some(Box::new(job));
        self.start_waiting();
    }

    /// Threads running now, for tests.
    pub fn running(&self) -> usize {
        self.state.lock().expect("scanner poisoned").running
    }

    fn start_waiting(&self) {
        // Taken and counted under one lock, so two finishing threads cannot both start past `max`.
        let job = {
            let mut s = self.state.lock().expect("scanner poisoned");
            if s.running >= self.max {
                return;
            }
            let Some(job) = s.waiting.take() else { return };
            s.running += 1;
            job
        };
        let me = self.clone();
        std::thread::spawn(move || {
            // Counted down even if the job panics, or the slot would be lost for good.
            struct Done(Scanner);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.state.lock().map(|mut s| s.running -= 1).ok();
                    self.0.start_waiting();
                }
            }
            let _done = Done(me);
            job();
        });
    }
}
