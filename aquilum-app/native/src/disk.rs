use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

type Job = Box<dyn FnOnce() + Send>;

static QUEUE: OnceLock<Sender<Job>> = OnceLock::new();

pub fn run(job: impl FnOnce() + Send + 'static) {
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            for job in rx {
                // A panic in one file operation must not terminate the global
                // serial disk worker and silently discard all subsequent writes.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                    eprintln!("background disk job panicked; worker recovered");
                }
            }
        });
        tx
    });
    let _ = queue.send(Box::new(job));
}

pub fn wait() {
    let (tx, rx) = channel();
    run(move || {
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn a_panicking_disk_job_does_not_discard_following_jobs() {
        let (tx, rx) = channel();
        super::run(|| panic!("intentional disk-job failure"));
        super::run(move || {
            let _ = tx.send("following job completed");
        });

        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), "following job completed");
        super::wait();
    }
}
