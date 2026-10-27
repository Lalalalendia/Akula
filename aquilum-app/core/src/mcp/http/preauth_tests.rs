use super::{Connection, Failure, IDLE_TIMEOUT, PREAUTH_TIMEOUT};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

#[test]
fn an_unauthenticated_trickle_cannot_occupy_a_slot_for_sixty_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut connection = Connection::new(&stream).unwrap();
    assert_eq!(PREAUTH_TIMEOUT, Duration::from_secs(5));
    assert_eq!(IDLE_TIMEOUT, Duration::from_secs(60));
    assert!(!connection.authenticated);
    let feeder = std::thread::spawn(move || {
        let _ = client.write_all(b"POST /");
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(100));
            if client.write_all(b"a").is_err() { break; }
        }
    });
    let started = Instant::now();
    assert!(matches!(connection.read_head(|_| true), Err(Failure::Gone)));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(4) && elapsed < Duration::from_secs(8),
        "pre-auth deadline was not enforced: {elapsed:?}");
    feeder.join().unwrap();
    connection.mark_authenticated();
    assert!(connection.authenticated, "only admitted connections get the longer budget");
}
