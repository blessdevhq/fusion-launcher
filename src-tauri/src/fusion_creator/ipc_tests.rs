//! Shared local IPC fixtures; these tests do not implement or call storage/job actions.
use super::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) {
    let typed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), *value);
    let mut extra = value.clone();
    extra["unknown"] = json!(true);
    assert!(serde_json::from_value::<T>(extra).is_err());
    for key in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<T>(missing).is_err(),
            "missing {key}"
        );
    }
}

#[test]
fn local_project_contracts_round_trip_shared_fixtures_with_required_nulls() {
    let f = fixture();
    round_trip::<CreatorProjectRequest>(&json!({"projectId": f["project"]["project_id"]}));
    round_trip::<CreatorProjectSaveRequest>(
        &json!({"project": f["project"], "expectedRevision": null}),
    );
    round_trip::<CreatorProjectSaveRequest>(
        &json!({"project": f["project"], "expectedRevision": 0}),
    );
    round_trip::<CreatorProjectSaveResponse>(&f["projectSaved"]);
    round_trip::<CreatorPrivateBindings>(&f["projectBindings"]["bindings"]);
    round_trip::<SavedPrivateBindings>(&f["projectBindings"]);
    round_trip::<CreatorProjectOpenResponse>(&json!({
        "project": f["project"], "privateBindings": f["projectBindings"], "missingInputs": []
    }));
    round_trip::<CreatorProjectOpenResponse>(&json!({
        "project": f["project"], "privateBindings": null, "missingInputs": f["projectMissingInputs"]
    }));
    round_trip::<CreatorProjectSummary>(&f["projectSummary"]);
    let list = json!([f["projectSummary"]]);
    let summaries: Vec<CreatorProjectSummary> = serde_json::from_value(list.clone()).unwrap();
    assert_eq!(serde_json::to_value(summaries).unwrap(), list);
    assert!(serde_json::from_value::<Vec<CreatorProjectSummary>>(json!([f["project"]])).is_err());
}

#[test]
fn project_save_rejects_legacy_duplicate_and_private_fields() {
    let f = fixture();
    let request = json!({"project": f["project"], "expectedRevision": null});
    let mut legacy = request.clone();
    legacy["project"] = f["legacyProject"].clone();
    assert!(serde_json::from_value::<CreatorProjectSaveRequest>(legacy).is_err());
    for field in [
        "adapter",
        "components",
        "build_profiles",
        "controls_profiles",
        "projection_rules",
        "privateBindings",
    ] {
        let mut duplicate = request.clone();
        duplicate["project"][field] = json!([]);
        assert!(
            serde_json::from_value::<CreatorProjectSaveRequest>(duplicate).is_err(),
            "{field}"
        );
    }
    let mut private = request;
    private["privateBindings"] = f["projectBindings"].clone();
    assert!(serde_json::from_value::<CreatorProjectSaveRequest>(private).is_err());
    let mut portable = f["project"].clone();
    portable["privateBindings"] = f["projectBindings"].clone();
    assert!(serde_json::from_value::<CreatorProgramProject>(portable).is_err());
    let manifest = json!({
        "program_descriptor": f["project"]["program_descriptor"], "privateBindings": f["projectBindings"]
    });
    assert!(
        serde_json::from_value::<crate::fusion_source::program_schema::ProgramCreatorManifest>(
            manifest
        )
        .is_err()
    );
}

#[test]
fn project_revisions_and_modified_time_remain_safe_json_integers() {
    let f = fixture();
    for invalid in [json!(-1), json!(0.5), json!(MAX_REVISION + 1), json!("0")] {
        let request = json!({"project": f["project"], "expectedRevision": invalid});
        assert!(serde_json::from_value::<CreatorProjectSaveRequest>(request).is_err());
        let mut summary = f["projectSummary"].clone();
        summary["modifiedAt"] = invalid.clone();
        assert!(serde_json::from_value::<CreatorProjectSummary>(summary).is_err());
        assert!(
            serde_json::from_value::<CreatorProjectSaveResponse>(json!({"revision": invalid}))
                .is_err()
        );
    }
    for valid in [0, MAX_REVISION] {
        let request = json!({"project": f["project"], "expectedRevision": valid});
        assert!(serde_json::from_value::<CreatorProjectSaveRequest>(request).is_ok());
        let mut summary = f["projectSummary"].clone();
        summary["modifiedAt"] = json!(valid);
        assert!(serde_json::from_value::<CreatorProjectSummary>(summary).is_ok());
    }
    for status in ["passed", "failed", "not_tested"] {
        let mut summary = f["projectSummary"].clone();
        summary["verificationStatus"] = json!(status);
        assert!(serde_json::from_value::<CreatorProjectSummary>(summary).is_ok());
    }
    let mut summary = f["projectSummary"].clone();
    summary["verificationStatus"] = json!("completed");
    assert!(serde_json::from_value::<CreatorProjectSummary>(summary).is_err());
}

#[test]
fn jobs_reports_and_export_use_descriptor_executor_not_legacy_adapter() {
    let f = fixture();
    round_trip::<CreatorResumeStamp>(&f["resumeStamp"]);
    round_trip::<CreatorReport>(&f["report"]);
    round_trip::<CreatorJobStartRequest>(&f["jobStart"]);
    for (name, value) in [("stamp", &f["resumeStamp"]), ("report", &f["report"])] {
        for keep_executor in [true, false] {
            let mut legacy = value.clone();
            if !keep_executor {
                legacy.as_object_mut().unwrap().remove("executor");
            }
            legacy["adapter"] = f["legacyProject"]["adapter"].clone();
            if name == "stamp" {
                assert!(serde_json::from_value::<CreatorResumeStamp>(legacy).is_err());
            } else {
                assert!(serde_json::from_value::<CreatorReport>(legacy).is_err());
            }
        }
    }
    let executor = &f["project"]["program_descriptor"]["executor"];
    assert_eq!(&f["resumeStamp"]["executor"], executor);
    assert_eq!(&f["report"]["executor"], executor);
}
