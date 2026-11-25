use super::*;

const EDEN: &str = include_str!("../../../../fixtures/creator/programs/eden-0.2.1.json");
const STK: &str = include_str!("../../../../fixtures/creator/programs/supertuxkart-1.5.json");

#[test]
fn program_schema_frozen_fixtures_typed_roundtrip() {
    for fixture in [EDEN, STK] {
        let descriptor: ProgramDescriptor = serde_json::from_str(fixture).unwrap();
        assert_eq!(
            serde_json::to_value(descriptor).unwrap(),
            serde_json::from_str::<serde_json::Value>(fixture).unwrap()
        );
    }
}
