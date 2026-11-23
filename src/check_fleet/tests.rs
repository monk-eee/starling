use super::*;
use crate::{
    config::{Config, StartOptions},
    conformance, fixture,
};
use serde_json::Value;
use tokio::net::TcpListener;

#[tokio::test]
async fn campaign_accepts_complete_evidence_once_and_rejects_spliced_reports() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("test process TLS provider");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = Config::demo(listener.local_addr().unwrap(), StartOptions::default()).unwrap();
    let revisions = vec![cfg.target.protocol];
    let report = fixture::alongside(listener, async {
        Ok(conformance::inspect(&cfg, &revisions, "wing").await)
    })
    .await
    .unwrap();
    let row = serde_json::to_value(report).unwrap();
    let (reports, _) = watch::channel(BTreeMap::new());
    let inbox = Inbox(Arc::new(Plan {
        campaign: "one".into(),
        expected: BTreeSet::from(["wing".into()]),
        revisions,
        reports,
        deadline: Instant::now() + Duration::from_secs(30),
        accepting: Mutex::new(true),
    }));
    let submit = |campaign: &str, v: &Value| {
        Request::new(CheckSubmission {
            campaign: campaign.into(),
            report_json: serde_json::to_vec(v).unwrap(),
        })
    };
    assert_eq!(
        inbox
            .submit(submit("other", &row))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    let mut unknown = row.clone();
    unknown["bird_id"] = json!("unlisted");
    assert_eq!(
        inbox
            .submit(submit("one", &unknown))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let mut missing = row.clone();
    missing["profiles"][0]["checks"]
        .as_array_mut()
        .unwrap()
        .pop();
    let mut forged = row.clone();
    forged["profiles"][0]["checks"][0]["status"] = json!("skip");
    let mut raw = row.clone();
    raw["unlisted_field"] = json!("PRIVATE_INPUT");
    for invalid in [missing, forged, raw] {
        assert_eq!(
            inbox
                .submit(submit("one", &invalid))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    assert!(inbox.0.reports.borrow().is_empty());
    inbox.submit(submit("one", &row)).await.unwrap();
    inbox.submit(submit("one", &row)).await.unwrap();
    assert_eq!(inbox.0.reports.borrow().len(), 1);
    let mut conflict = row;
    conflict["pid"] = json!(conflict["pid"].as_u64().unwrap() + 1);
    assert_eq!(
        inbox
            .submit(submit("one", &conflict))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::AlreadyExists
    );
}
