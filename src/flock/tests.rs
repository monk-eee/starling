use super::*;

fn pulse(incarnation: u64, sequence: u64, requests: u64) -> Heartbeat {
    Heartbeat {
        bird_id: "wing-1".into(),
        incarnation,
        sequence,
        pid: 7,
        state: "running".into(),
        requests,
        p95_ms: Some(12.0),
        ..Default::default()
    }
}

#[test]
fn reconnects_do_not_duplicate_birds_or_freshen_replayed_counts() {
    let flock = Flock::default();
    let at = Instant::now();
    flock.record(pulse(1, 2, 8), at).unwrap();
    flock.record(pulse(1, 1, 3), at + STALE).unwrap();
    let birds = flock.view(at + STALE);
    assert_eq!(birds.len(), 1);
    assert!(birds[0].stale);
    assert_eq!(birds[0].requests, 8);
    flock.record(pulse(1, 3, 10), at + STALE).unwrap();
    assert!(!flock.view(at + STALE)[0].stale);
    assert_eq!(
        flock.record(pulse(2, 1, 0), at + STALE).unwrap_err().code(),
        tonic::Code::AlreadyExists
    );
    flock.record(pulse(2, 1, 0), at + 2 * STALE).unwrap();
    assert_eq!(flock.view(at + 2 * STALE)[0].requests, 0);
    assert!(flock.view(at + 2 * STALE + RETAIN).is_empty());
}
