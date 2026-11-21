use super::*;

fn sample() -> Config {
    toml::from_str(include_str!("../../examples/local.toml")).unwrap()
}

#[test]
fn malformed_acceptance_criteria_cannot_silently_disable_the_gate() {
    for bad in [
        "{}",
        "null",
        "['< 0.1%']",
        "{p90: '< 2s'}",
        "{p95: null, min_requests: 1}",
        "{p99: '500ms'}",
        "{p95: '< 500'}",
        "{p95: '> 1s'}",
        "{p99: '< -2ms'}",
        "{error_rate: '< 0.1'}",
        "{error_rate: '< 101%'}",
        "{error_rate: '< -1%'}",
        "{error_rate: '< NaN%'}",
        "{error_rate: '< 0.1234567%'}",
        "{min_requests: 0}",
        "{min_requests: -1}",
        "{min_requests: 1.5}",
    ] {
        let yaml = format!("{STARTER_YAML}\nassert: {bad}\n");
        assert!(
            Config::from_yaml(&yaml, Default::default()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn optional_wait_accepts_zero_and_preserves_explicit_durations_in_both_formats() {
    for (value, ms) in [
        (None, 0),
        (Some("0"), 0),
        (Some("\"0ms\""), 0),
        (Some("\"125ms\""), 125),
    ] {
        let yaml = format!(
            "users: 1\ntimeout: 3s\n{}",
            value.map(|s| format!("wait: {s}")).unwrap_or_default()
        );
        let toml = format!(
            "users = 1\ntimeout = \"3s\"\n{}",
            value.map(|s| format!("wait = {s}")).unwrap_or_default()
        );
        for load in [
            serde_yaml_ng::from_str::<Load>(&yaml).unwrap(),
            toml::from_str::<Load>(&toml).unwrap(),
        ] {
            let mut cfg = sample();
            cfg.load = load;
            cfg.validate().unwrap();
            assert_eq!(cfg.load.wait, Duration::from_millis(ms));
        }
    }
    for invalid in ["-1", "250", "\"-1s\""] {
        assert!(
            toml::from_str::<Load>(&format!("users = 1\ntimeout = \"3s\"\nwait = {invalid}"))
                .is_err()
        );
    }
}

#[test]
fn local_limits_and_weights_are_checked_before_dispatch() {
    for url in [
        "http://192.0.2.1/mcp",
        "http://localhost.example/mcp",
        "http://u:p@localhost/mcp",
        "http://localhost/mcp?token=abc",
    ] {
        let mut cfg = sample();
        cfg.target.url = url.parse().unwrap();
        assert!(cfg.validate().is_err());
    }
    let mut cfg = sample();
    cfg.target.url = "http://localhost:9000/mcp".parse().unwrap();
    cfg.validate().unwrap();
    assert_eq!(cfg.target.url.host_str(), Some("127.0.0.1"));
    cfg.tasks[0].weight = 0;
    assert!(cfg.validate().is_err());
    assert!(loopback_addr("0.0.0.0:9000").is_err());
}

#[test]
fn request_validation_rejects_ambiguous_cases_and_checks_even_unused_targets() {
    let mut cfg = sample();
    cfg.tasks.push(cfg.tasks[0].clone());
    assert!(cfg.validate().is_err());
    cfg.tasks.last_mut().unwrap().name = Some("repeat".into());
    cfg.validate().unwrap();
    cfg.tasks[0].method = Method::ListTools;
    assert!(cfg.validate().is_err()); // A list must not silently discard tool arguments.
    let mut cfg = sample();
    cfg.target.endpoints.insert(
        "other".into(),
        "http://example.invalid/mcp".parse().unwrap(),
    );
    assert!(cfg.validate().is_err());
    cfg.target.endpoints.insert(
        "other".into(),
        "http://localhost:9000/private-path".parse().unwrap(),
    );
    cfg.target.selected = Some("other".into());
    cfg.validate().unwrap();
    assert_eq!(
        cfg.target.endpoint().as_str(),
        "http://127.0.0.1:9000/private-path"
    );
    cfg.target.selected = Some("absent".into());
    assert!(cfg.validate().is_err());
}

#[test]
fn selection_is_seeded_and_respects_weights() {
    let cfg = sample();
    let sequence = || {
        let mut rng = fastrand::Rng::with_seed(42);
        (0..1000)
            .map(|_| cfg.pick(&mut rng).tool.as_deref().unwrap())
            .collect::<Vec<_>>()
    };
    let draws = sequence();
    assert_eq!(draws, sequence());
    assert!((680..820).contains(&draws.iter().filter(|&&s| s == "echo").count()));
}
