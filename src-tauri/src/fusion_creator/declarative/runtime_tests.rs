use super::*;
use crate::fusion_source::program_schema::ProgramDescriptor;
use serde_json::json;

fn descriptor() -> ProgramDescriptor {
    ProgramDescriptor::read_json(&json!({
        "document_type":"program_descriptor", "schema_version":"1.0",
        "identity":{"id":"generic","title":"Generic","version":"1", "os":"windows","architecture":"x86_64"},
        "executor":{"id":"fusion.declarative","version":"1.0"},
        "components":[{"id":"runtime","layer_id":"runtime","origin":{"kind":"managed","artifact_ref":"runtime","redistribution_allowed":false},"source_root":"","mount_path":"app"}],
        "required_files":[{"path":"app/program.exe","role":"main_executable"}],
        "projection_rules":[],"directories":["state"],
        "entrypoint":{"executable":"app/program.exe","working_directory":"app","arguments":[{"kind":"literal","value":"{{NOT_A_TEMPLATE}} & whoami"},{"kind":"workspace_path","path":"state","prefix":"--data="}],"environment_paths":[{"name":"GENERIC_DATA_DIR","path":"state"}]},
        "configs":[],"build_profiles":[{"id":"default","title":"Default","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
        "controls_profiles":[{"id":"native","title":"Defaults","kind":"application_defaults"}],"persistence":[]
    }).to_string()).unwrap()
}

#[test]
fn program_argv_is_literal_and_workspace_path_is_one_atom_with_explicit_cwd() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("app")).unwrap();
    std::fs::create_dir(temp.path().join("state")).unwrap();
    let program = descriptor();
    let approved = BTreeSet::from(["GENERIC_DATA_DIR".to_owned()]);
    let command =
        prepare_program_command(&program, temp.path(), &BTreeMap::new(), &approved).unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        vec![
            "{{NOT_A_TEMPLATE}} & whoami".to_owned(),
            format!("--data={}", temp.path().join("state").display())
        ]
    );
    assert_eq!(
        command.get_current_dir(),
        Some(temp.path().join("app").as_path())
    );
    assert_eq!(
        command.get_program(),
        temp.path().join("app/program.exe").as_os_str()
    );
    assert_eq!(
        command
            .get_envs()
            .find(|(k, _)| *k == "GENERIC_DATA_DIR")
            .unwrap()
            .1
            .unwrap(),
        temp.path().join("state").as_os_str()
    );
}

#[test]
fn undeclared_local_environment_approval_is_not_inferred_from_name() {
    let temp = tempfile::tempdir().unwrap();
    let error = prepare_program_command(
        &descriptor(),
        temp.path(),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::ActionUnavailable);
}
