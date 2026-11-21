use super::*;

#[test]
fn week_timeline_stays_bounded_and_commits_each_live_sample_once() {
    let week = Duration::from_secs(7 * 86400);
    let mut m = Metrics::new(week);
    let dt = Duration::from_secs;
    m.record("tools/call/sample", dt(1), dt(1), Outcome::Success);
    m.seal(dt(5), false);
    let live = m.open_interval(dt(5)).unwrap();
    assert_eq!((live.second, live.span_ms, live.calls), (0, 5000.0, 1));
    assert!(m.series.is_empty());

    m.record("tools/call/sample", dt(336), dt(1), Outcome::Success);
    m.seal(dt(336), false);
    assert_eq!(m.series[0].calls, 1);
    assert!(m.open_interval(dt(336)).is_none());
    assert_eq!(m.open_interval(dt(337)).unwrap().second, 336);
    m.record("tools/call/sample", week + dt(4), dt(2), Outcome::Timeout);
    // Even a suspended machine must not allocate a backlog of empty intervals on wake.
    m.seal(week + dt(86400), true);
    assert_eq!(m.series.len(), 1801);
    let last = m.series.last().unwrap();
    assert_eq!(
        (last.second, last.span_ms, last.failed),
        (604800, 86400000.0, 1)
    );
    assert_eq!(m.series.iter().map(|p| p.calls).sum::<u64>(), 3);
    assert_eq!(m.summarize().0.count, 3);
    assert!(m.series[2].p95_ms.is_none());
}

#[test]
fn merge_samples_not_percentiles_and_keep_final_partial_bucket() {
    let mut m = Metrics::new(Duration::from_secs(3));
    let ms = Duration::from_millis;
    m.record("initialize", ms(5), ms(5), Outcome::Success);
    for _ in 0..100 {
        m.record("tools/call/fast", ms(999), ms(1), Outcome::Success);
    }
    m.record("tools/call/slow", ms(2100), ms(901), Outcome::ToolError);
    m.seal(ms(2100), true);
    let (all, ops) = m.summarize();
    assert_eq!(all.count, 101);
    assert_eq!(all.outcomes.values().sum::<u64>(), all.count);
    assert_eq!(all.failed, 1);
    assert!(all.p95_ms.unwrap() < 2.0);
    assert!(ops["tools/call/slow"].p95_ms.unwrap() >= 901.0);
    assert_eq!(
        m.series.iter().map(|p| p.calls).collect::<Vec<_>>(),
        [100, 0, 1]
    );
    assert_eq!(m.series[1].p50_ms, None);
    assert_eq!(m.series[2].span_ms, 100.0);
}
