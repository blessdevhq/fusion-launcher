use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use sha2::Digest;

pub const EXECUTION_PLAN_VERSION: u32 = 1;

/// Provenance of the expected hash used to verify downloaded or imported assets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HashOrigin {
    /// Pinned by the bundled/official profile registry (trusted for executables).
    Registry,
    /// Declared in an external manifest (untrusted for executables).
    Manifest,
    /// Computed from local payload.
    Computed,
}

/// A validated, idempotent plan to bring a game and its environment
/// into the `ReadyToPlay` state.
///
/// NOTE: The plan strictly describes preparatory actions. It does NOT
/// launch the game process or manage process lifecycle (which is handled
/// separately by `launcher::launch_game` and `LaunchSession`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub version: u32,
    pub game_id: String,
    pub fingerprint: String,
    pub steps: Vec<ExecutionStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStep {
    pub id: String,
    pub action: StepAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepAction {
    /// Ensure the required emulator is installed via the trusted profile registry.
    EnsureEmulator {
        platform: String,
        profile_id: Option<String>,
    },
    /// Fetch an artifact from a remote source into managed staging.
    FetchArtifact {
        artifact_id: String,
        label: String,
        expected_sha256: Option<String>,
        hash_origin: HashOrigin,
        is_executable: bool,
    },
    /// Verify the integrity and provenance of an artifact.
    VerifyArtifact {
        artifact_id: String,
        sha256: String,
    },
    /// Extract a verified archive into its target managed directory.
    ExtractArtifact {
        artifact_id: String,
        destination_label: String,
    },
    /// Check whether platform system files (BIOS, firmware, keys) are present.
    CheckSystemRequirements {
        platform: String,
        required_identifiers: Vec<String>,
    },
    /// Apply platform or game configuration templates.
    ApplyConfiguration {
        platform: String,
        profile_id: String,
    },
    /// Final readiness verification. Must be the last step of the plan.
    VerifyReadiness {
        game_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion(u32),
    EmptyGameId,
    EmptyFingerprint,
    EmptySteps,
    DuplicateStepId(String),
    MissingFinalReadinessStep,
    ReadinessNotFinalStep(usize),
    ExecutableRequiresRegistryHash(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "unsupported_plan_version:{v}"),
            Self::EmptyGameId => write!(f, "plan_game_id_empty"),
            Self::EmptyFingerprint => write!(f, "plan_fingerprint_empty"),
            Self::EmptySteps => write!(f, "plan_steps_empty"),
            Self::DuplicateStepId(id) => write!(f, "duplicate_step_id:{id}"),
            Self::MissingFinalReadinessStep => write!(f, "plan_missing_final_readiness_step"),
            Self::ReadinessNotFinalStep(idx) => {
                write!(f, "readiness_step_must_be_last_at_index:{idx}")
            }
            Self::ExecutableRequiresRegistryHash(id) => {
                write!(f, "executable_requires_registry_hash:{id}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl ExecutionPlan {
    /// Validates internal consistency, required fields, and step order invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != EXECUTION_PLAN_VERSION {
            return Err(ValidationError::UnsupportedVersion(self.version));
        }
        if self.game_id.trim().is_empty() {
            return Err(ValidationError::EmptyGameId);
        }
        if self.fingerprint.trim().is_empty() {
            return Err(ValidationError::EmptyFingerprint);
        }
        if self.steps.is_empty() {
            return Err(ValidationError::EmptySteps);
        }

        let mut seen_ids = HashSet::with_capacity(self.steps.len());
        let last_idx = self.steps.len() - 1;

        for (idx, step) in self.steps.iter().enumerate() {
            if !seen_ids.insert(step.id.as_str()) {
                return Err(ValidationError::DuplicateStepId(step.id.clone()));
            }

            // Security invariant: executables must have pinned hash originating from Registry.
            if let StepAction::FetchArtifact {
                artifact_id,
                expected_sha256,
                hash_origin,
                is_executable,
                ..
            } = &step.action
            {
                if *is_executable
                    && (*hash_origin != HashOrigin::Registry || expected_sha256.is_none())
                {
                    return Err(ValidationError::ExecutableRequiresRegistryHash(
                        artifact_id.clone(),
                    ));
                }
            }

            let is_readiness = matches!(step.action, StepAction::VerifyReadiness { .. });
            if is_readiness && idx != last_idx {
                return Err(ValidationError::ReadinessNotFinalStep(idx));
            }
        }

        match self.steps.last().map(|s| &s.action) {
            Some(StepAction::VerifyReadiness { .. }) => Ok(()),
            _ => Err(ValidationError::MissingFinalReadinessStep),
        }
    }
}

pub struct PlanResolver;

impl PlanResolver {
    pub fn resolve_plan(
        game: &crate::schema::CatalogGameView,
        setup_state: &crate::schema::GameSetupState,
    ) -> Result<ExecutionPlan, String> {
        let mut steps = Vec::new();
        let mut step_num = 1;

        // 1. Ensure Emulator if not ready
        if setup_state.emulator.status != "ready" {
            steps.push(ExecutionStep {
                id: format!("step-{step_num}-ensure-emulator"),
                action: StepAction::EnsureEmulator {
                    platform: game.platform.clone(),
                    profile_id: setup_state.profile_id.clone(),
                },
            });
            step_num += 1;
        }

        // 2. Check system requirements if any required files are missing/corrupt
        let required_sys_files: Vec<String> = setup_state
            .system_files
            .iter()
            .filter(|f| f.required && f.status != "ready")
            .map(|f| f.id.clone())
            .collect();
        if !required_sys_files.is_empty() {
            steps.push(ExecutionStep {
                id: format!("step-{step_num}-check-sys-requirements"),
                action: StepAction::CheckSystemRequirements {
                    platform: game.platform.clone(),
                    required_identifiers: required_sys_files,
                },
            });
            step_num += 1;
        }

        // 3. Game artifact payload if missing or corrupt
        if setup_state.game_file.status != "ready" {
            let artifact_id = format!("{}-payload", game.id);
            steps.push(ExecutionStep {
                id: format!("step-{step_num}-fetch-game"),
                action: StepAction::FetchArtifact {
                    artifact_id: artifact_id.clone(),
                    label: game.title.clone(),
                    expected_sha256: None,
                    hash_origin: HashOrigin::Computed,
                    is_executable: false,
                },
            });
            step_num += 1;
        }

        // 4. Always ends strictly with VerifyReadiness
        steps.push(ExecutionStep {
            id: format!("step-{step_num}-verify-readiness"),
            action: StepAction::VerifyReadiness {
                game_id: game.id.clone(),
            },
        });

        // Compute deterministic fingerprint
        let mut hasher = sha2::Sha256::new();
        sha2::Digest::update(&mut hasher, game.id.as_bytes());
        sha2::Digest::update(&mut hasher, game.platform.as_bytes());
        for s in &steps {
            sha2::Digest::update(&mut hasher, s.id.as_bytes());
        }
        let fingerprint = format!("{:x}", sha2::Digest::finalize(hasher))[..16].to_string();

        let plan = ExecutionPlan {
            version: EXECUTION_PLAN_VERSION,
            game_id: game.id.clone(),
            fingerprint,
            steps,
        };

        plan.validate()
            .map_err(|e| format!("Failed to build valid plan: {e}"))?;

        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_plan() -> ExecutionPlan {
        ExecutionPlan {
            version: EXECUTION_PLAN_VERSION,
            game_id: "snes::chrono-trigger".to_string(),
            fingerprint: "fp-12345".to_string(),
            steps: vec![
                ExecutionStep {
                    id: "step-1-emu".to_string(),
                    action: StepAction::EnsureEmulator {
                        platform: "snes".to_string(),
                        profile_id: Some("snes9x".to_string()),
                    },
                },
                ExecutionStep {
                    id: "step-2-ready".to_string(),
                    action: StepAction::VerifyReadiness {
                        game_id: "snes::chrono-trigger".to_string(),
                    },
                },
            ],
        }
    }

    #[test]
    fn valid_plan_passes_validation() {
        assert!(valid_plan().validate().is_ok());
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut plan = valid_plan();
        plan.version = 99;
        assert_eq!(
            plan.validate(),
            Err(ValidationError::UnsupportedVersion(99))
        );
    }

    #[test]
    fn rejects_empty_game_id() {
        let mut plan = valid_plan();
        plan.game_id = "   ".to_string();
        assert_eq!(plan.validate(), Err(ValidationError::EmptyGameId));
    }

    #[test]
    fn rejects_duplicate_step_ids() {
        let mut plan = valid_plan();
        plan.steps[1].id = plan.steps[0].id.clone();
        assert_eq!(
            plan.validate(),
            Err(ValidationError::DuplicateStepId("step-1-emu".to_string()))
        );
    }

    #[test]
    fn requires_verify_readiness_as_final_step() {
        let mut plan = valid_plan();
        plan.steps.pop();
        assert_eq!(
            plan.validate(),
            Err(ValidationError::MissingFinalReadinessStep)
        );
    }

    #[test]
    fn rejects_verify_readiness_in_middle_of_plan() {
        let mut plan = valid_plan();
        plan.steps.push(ExecutionStep {
            id: "step-3-trailing".to_string(),
            action: StepAction::ApplyConfiguration {
                platform: "snes".to_string(),
                profile_id: "snes9x".to_string(),
            },
        });
        assert_eq!(
            plan.validate(),
            Err(ValidationError::ReadinessNotFinalStep(1))
        );
    }

    #[test]
    fn rejects_launch_game_action_deserialization() {
        let json = r#"{
            "kind": "launch_game",
            "profileId": "snes9x"
        }"#;
        let result: Result<StepAction, _> = serde_json::from_str(json);
        assert!(result.is_err(), "StepAction must not allow launch_game");
    }

    #[test]
    fn rejects_executable_artifact_from_manifest_or_without_hash() {
        let mut plan = valid_plan();
        // Insert untrusted executable step before readiness
        plan.steps.insert(
            1,
            ExecutionStep {
                id: "step-bad-exe".to_string(),
                action: StepAction::FetchArtifact {
                    artifact_id: "manifest::bad_exe".to_string(),
                    label: "Bad Emulator".to_string(),
                    expected_sha256: Some("a".repeat(64)),
                    hash_origin: HashOrigin::Manifest,
                    is_executable: true,
                },
            },
        );
        assert_eq!(
            plan.validate(),
            Err(ValidationError::ExecutableRequiresRegistryHash(
                "manifest::bad_exe".to_string()
            ))
        );

        // Even with None hash
        if let StepAction::FetchArtifact {
            hash_origin,
            expected_sha256,
            ..
        } = &mut plan.steps[1].action
        {
            *hash_origin = HashOrigin::Registry;
            *expected_sha256 = None;
        }
        assert_eq!(
            plan.validate(),
            Err(ValidationError::ExecutableRequiresRegistryHash(
                "manifest::bad_exe".to_string()
            ))
        );

        // Fixed with Registry origin and pinned hash
        if let StepAction::FetchArtifact {
            expected_sha256, ..
        } = &mut plan.steps[1].action
        {
            *expected_sha256 = Some("a".repeat(64));
        }
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn plan_resolver_generates_valid_plan_for_missing_emulator_and_game() {
        let game = crate::schema::CatalogGameView {
            id: "nes::mario".to_string(),
            source_id: "mario".to_string(),
            repository_id: "repo".to_string(),
            repository_name: "Repository".to_string(),
            platform: "nes".to_string(),
            title: "Super Mario Bros".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: None,
            setup_profile_id: Some("nes-mesen".to_string()),
            downloads: vec![],
            expected_extensions: vec![".nes".to_string()],
            required_system_file_ids: vec![],
            launch: None,
        };

        let setup_state = crate::schema::GameSetupState {
            game_id: "nes::mario".to_string(),
            profile_id: Some("nes-mesen".to_string()),
            profile_display_name: Some("NES / Mesen2".to_string()),
            unsupported_profile_id: None,
            emulator: crate::schema::GameSetupEmulatorState {
                status: "missing".to_string(),
                profile_id: Some("nes-mesen".to_string()),
                platform: "nes".to_string(),
                emulator_name: "Mesen2".to_string(),
                install_mode: "downloadable".to_string(),
                executable_path: None,
                message: None,
            },
            system_files: vec![],
            repository_requirements: vec![],
            game_file: crate::schema::GameSetupGameFileState {
                status: "missing".to_string(),
                installed_path: None,
                expected_extensions: vec![".nes".to_string()],
                allow_directory: false,
                message: None,
            },
            launch: crate::schema::GameSetupLaunchState {
                status: "blocked".to_string(),
                blockers: vec!["Emulator missing".to_string()],
            },
            primary_action: "Install".to_string(),
        };

        let plan = PlanResolver::resolve_plan(&game, &setup_state).unwrap();
        assert_eq!(plan.version, EXECUTION_PLAN_VERSION);
        assert_eq!(plan.game_id, "nes::mario");
        assert_eq!(plan.steps.len(), 3);
        assert!(matches!(plan.steps[0].action, StepAction::EnsureEmulator { .. }));
        assert!(matches!(plan.steps[1].action, StepAction::FetchArtifact { .. }));
        assert!(matches!(plan.steps[2].action, StepAction::VerifyReadiness { .. }));
    }

    #[test]
    fn plan_resolver_for_fully_ready_game_only_needs_readiness() {
        let game = crate::schema::CatalogGameView {
            id: "nes::mario".to_string(),
            source_id: "mario".to_string(),
            repository_id: "repo".to_string(),
            repository_name: "Repository".to_string(),
            platform: "nes".to_string(),
            title: "Super Mario Bros".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: None,
            setup_profile_id: Some("nes-mesen".to_string()),
            downloads: vec![],
            expected_extensions: vec![".nes".to_string()],
            required_system_file_ids: vec![],
            launch: None,
        };

        let setup_state = crate::schema::GameSetupState {
            game_id: "nes::mario".to_string(),
            profile_id: Some("nes-mesen".to_string()),
            profile_display_name: Some("NES / Mesen2".to_string()),
            unsupported_profile_id: None,
            emulator: crate::schema::GameSetupEmulatorState {
                status: "ready".to_string(),
                profile_id: Some("nes-mesen".to_string()),
                platform: "nes".to_string(),
                emulator_name: "Mesen2".to_string(),
                install_mode: "downloadable".to_string(),
                executable_path: Some("C:/Emus/Mesen.exe".to_string()),
                message: None,
            },
            system_files: vec![],
            repository_requirements: vec![],
            game_file: crate::schema::GameSetupGameFileState {
                status: "ready".to_string(),
                installed_path: Some("C:/Games/mario.nes".to_string()),
                expected_extensions: vec![".nes".to_string()],
                allow_directory: false,
                message: None,
            },
            launch: crate::schema::GameSetupLaunchState {
                status: "ready".to_string(),
                blockers: vec![],
            },
            primary_action: "Launch".to_string(),
        };

        let plan = PlanResolver::resolve_plan(&game, &setup_state).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert!(matches!(plan.steps[0].action, StepAction::VerifyReadiness { .. }));
    }
}
