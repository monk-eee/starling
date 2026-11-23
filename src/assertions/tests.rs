use super::*;
use serde_json::json;

fn rule() -> MatchAssertion {
    serde_json::from_value(json!({
        "label":"Expected public marker from the fixture", "path":"/results", "first":2,
        "all":[{"path":"/source","matches":["^fixture$"]},
               {"path":"/body","markdown":true,"matches":["\\bready\\b","\\bvalue\\b"]}]
    }))
    .unwrap()
}

#[test]
fn predicates_require_one_in_scope_nonempty_passage_within_the_rank_cutoff() {
    let r = rule();
    r.validate().unwrap();
    let miss = json!({"source":"elsewhere","body":"ready value"});
    let url_only = json!({"source":"fixture","body":"[open](https://example.test/ready/value)"});
    let hit = json!({"source":"fixture","body":"`READY`   value"});
    for value in [
        json!({}),
        json!({"results":null}),
        json!({"results":[]}),
        json!({"results":[miss,url_only,hit]}),
        json!({"results":[{"source":"fixture","body":"ready"},{"source":"fixture","body":"value"}]}),
    ] {
        assert_eq!(r.rank(&value), None);
    }
    assert_eq!(r.rank(&json!({"results":[miss,hit]})), Some(2));
}

#[test]
fn invalid_or_vacuous_predicates_do_not_disable_an_assertion() {
    for (field, value) in [
        ("first", json!(0)),
        ("path", json!("results")),
        ("label", json!("")),
        ("all", json!([])),
        ("all", json!([{"path":"/body","matches":[]}])),
        ("all", json!([{"path":"/body","matches":[".*"]}])),
        ("all", json!([{"path":"/body","matches":["["]}])),
    ] {
        let mut v = json!({"path":"/results","label":"public","first":1,
            "all":[{"path":"/body","matches":["expected"]}]});
        v[field] = value;
        assert!(
            serde_json::from_value::<MatchAssertion>(v).map_or(true, |r| r.validate().is_err())
        );
    }
}
