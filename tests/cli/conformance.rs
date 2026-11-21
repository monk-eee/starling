use super::*;
use std::collections::BTreeSet;

#[tokio::test]
async fn fleet_checks_each_revision_in_each_independent_process() {
    let work = tempdir().unwrap();
    let out = timeout(
        Duration::from_secs(30),
        starling()
            .current_dir(work.path())
            .args(["collect", "--demo-birds", "2", "--report", "fleet"])
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dir = work.path().join("fleet");
    let report = read(&dir, "fleet-conformance.json");
    assert_eq!(report["verdict"], "pass");
    assert_eq!(report["finish_reason"], "complete");
    assert!(report["missing_birds"].as_array().unwrap().is_empty());
    let workers = report["workers"].as_object().unwrap();
    let mut pids = BTreeSet::new();
    assert_eq!(workers.len(), 2);
    for (id, worker) in workers {
        assert!(pids.insert(worker["pid"].as_u64().unwrap()));
        assert_eq!(
            worker,
            &read(&dir.join("birds").join(id), "conformance.json")
        );
        let profiles = worker["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 4);
        for p in profiles {
            assert_eq!(p["verdict"], "pass");
            assert_eq!(p["requested"], p["negotiated"]);
            assert!(
                p["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["rule"] == "catalog.resources" && r["status"] == "skip")
            );
        }
    }
}

#[tokio::test]
async fn an_absent_expected_worker_cannot_produce_a_passing_campaign() {
    let work = tempdir().unwrap();
    let mut collector = starling()
        .current_dir(work.path())
        .args([
            "collect",
            "--listen",
            "127.0.0.1:0",
            "--expect",
            "here,missing",
            "--protocol",
            "2025-06-18",
            "--deadline-secs",
            "4",
            "--report",
            "fleet",
        ])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(collector.stdout.take().unwrap()).lines();
    let home = timeout(
        Duration::from_secs(8),
        announced(&mut lines, "Check collector: "),
    )
    .await
    .unwrap();
    let campaign = announced(&mut lines, "Check campaign: ").await;
    let result = starling()
        .current_dir(work.path())
        .args([
            "check",
            "--demo",
            "--protocol",
            "2025-06-18",
            "--id",
            "here",
            "--home",
            &home,
            "--campaign",
            &campaign,
            "--report",
            "here",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        timeout(Duration::from_secs(10), collector.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(1)
    );
    let fleet = read(&work.path().join("fleet"), "fleet-conformance.json");
    assert_eq!(fleet["missing_birds"], serde_json::json!(["missing"]));
    assert_eq!(fleet["workers"]["here"]["verdict"], "pass");
    assert_eq!(fleet["finish_reason"], "deadline");
    assert_eq!(fleet["verdict"], "inconclusive");
}
