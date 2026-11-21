use super::*;
use crate::acceptance::{Criteria, GateStatus};

#[test]
fn performance_gate_uses_merged_workload_samples_and_exact_boundaries() {
    let ms = Duration::from_millis;
    let mut m = Metrics::new(ms(1000));
    for _ in 0..970 {
        m.record("tools/call/fast", ms(1), ms(1), Outcome::Success);
    }
    for _ in 0..29 {
        m.record("task/prompts/get/slow", ms(1), ms(2), Outcome::Success);
    }
    m.record("task/prompts/get/slow", ms(1), ms(2), Outcome::Assertion);
    // Setup errors and their very long latencies must not change the workload gate.
    for _ in 0..100 {
        m.record("initialize", ms(1), ms(800), Outcome::Timeout);
    }
    let calls = m.summarize().0;
    for (op, passes) in [("<", false), ("<=", true)] {
        let criteria: Criteria = toml::from_str(&format!(
            "error_rate = '{op} 0.1%'\np95 = '{op} 1ms'\np99 = '{op} 2ms'\nmin_requests = 1000"
        ))
        .unwrap();
        let result = criteria.evaluate(&calls, true);
        assert_eq!(result.status == GateStatus::Passed, passes);
        let rows: Vec<_> = result.checks.iter().map(|c| (c.metric, c.passed)).collect();
        assert_eq!(
            rows,
            [
                ("error_rate", Some(passes)),
                ("p95", Some(passes)),
                ("p99", Some(passes)),
                ("min_requests", Some(true))
            ]
        );
        let partial = criteria.evaluate(&calls, false);
        assert!(
            partial.status == GateStatus::Incomplete
                && partial.checks.iter().all(|c| c.passed.is_none())
        );
    }
}

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
