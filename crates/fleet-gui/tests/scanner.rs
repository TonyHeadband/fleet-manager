use fleet_gui::components::Scanner;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn wait_until(mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A job that blocks, like a read on a hung mount, until `release` gets a message.
fn hung(release: &Arc<Mutex<Receiver<()>>>) -> impl FnOnce() + Send + 'static {
    let release = release.clone();
    move || {
        let _ = release.lock().unwrap().recv();
    }
}

#[test]
fn hung_reads_never_take_more_than_the_cap() {
    let s = Scanner::new(2);
    let (tx, rx) = channel();
    let release = Arc::new(Mutex::new(rx));
    for _ in 0..10 {
        s.run(hung(&release));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(s.running(), 2);
    for _ in 0..10 {
        let _ = tx.send(());
    }
    wait_until(|| s.running() == 0);
}

#[test]
fn only_the_newest_waiting_job_runs_once_a_slot_frees() {
    let s = Scanner::new(1);
    let (tx, rx) = channel();
    let release = Arc::new(Mutex::new(rx));
    s.run(hung(&release));
    wait_until(|| s.running() == 1);
    let ran = Arc::new(Mutex::new(Vec::new()));
    for name in ["older", "newer", "newest"] {
        let ran = ran.clone();
        s.run(move || ran.lock().unwrap().push(name));
    }
    assert!(ran.lock().unwrap().is_empty(), "started past the cap");
    tx.send(()).unwrap();
    wait_until(|| s.running() == 0 && !ran.lock().unwrap().is_empty());
    assert_eq!(*ran.lock().unwrap(), ["newest"]);
}

#[test]
fn a_panicking_job_gives_its_slot_back() {
    let s = Scanner::new(1);
    s.run(|| panic!("scan blew up"));
    wait_until(|| s.running() == 0);
    let (tx, rx) = channel();
    s.run(move || tx.send(()).unwrap());
    rx.recv_timeout(Duration::from_secs(5)).expect("the slot was lost");
}
