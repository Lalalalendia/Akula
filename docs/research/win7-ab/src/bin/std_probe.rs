//! Force clock, Condvar, thread and TLS machinery into the linked executable.
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

fn main() {
    let before = SystemTime::now();
    let shared = Arc::new((Mutex::new(false), Condvar::new()));
    let worker = Arc::clone(&shared);
    let thread = std::thread::spawn(move || {
        let (lock, ready) = &*worker;
        *lock.lock().expect("lock poisoned") = true;
        ready.notify_one();
    });
    let (lock, ready) = &*shared;
    let (done, _) = ready
        .wait_timeout_while(lock.lock().expect("lock poisoned"), Duration::from_secs(5), |done| !*done)
        .expect("wait failed");
    assert!(*done);
    thread.join().expect("worker failed");
    println!("std-ok {:?}", before.elapsed().expect("time moved backward"));
}
