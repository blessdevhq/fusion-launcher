use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn list_emulator_configs(&self) -> Result<Vec<EmulatorConfig>, String> {
        Ok(self
            .list_profile_emulator_configs()?
            .into_iter()
            .filter(|config| {
                setup_profiles::get_default_platform_setup_profile(&config.platform)
                    .map(|profile| profile.id == config.profile_id)
                    .unwrap_or(false)
            })
            .map(emulator_config_from_profile)
            .collect())
    }

    pub fn get_emulator_config(&self, platform: &str) -> Result<Option<EmulatorConfig>, String> {
        let Some(profile) = setup_profiles::get_default_platform_setup_profile(platform) else {
            return Ok(None);
        };
        Ok(self
            .get_profile_emulator_config(&profile.id)?
            .map(emulator_config_from_profile))
    }

    pub fn get_emulator_exe_path(
        &self,
        platform: &str,
        profile_id: Option<&str>,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let path = match profile_id {
            Some(profile_id) => self
                .get_profile_emulator_config(profile_id)?
                .filter(|config| config.status == "valid")
                .and_then(|config| config.exe_path),
            None => self
                .get_emulator_config(platform)?
                .filter(|config| config.status == "valid")
                .and_then(|config| config.exe_path),
        };

        Ok(path
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file()))
    }

    pub fn is_emulator_installed(
        &self,
        platform: &str,
        profile_id: Option<&str>,
    ) -> Result<bool, String> {
        Ok(self.get_emulator_exe_path(platform, profile_id)?.is_some())
    }

    pub fn upsert_emulator_config(
        &self,
        platform: &str,
        exe_path: Option<&str>,
        status: &str,
        version: Option<&str>,
        launch_args_template: Option<&str>,
    ) -> Result<EmulatorConfig, String> {
        let profile = setup_profiles::get_default_platform_setup_profile(platform)
            .ok_or_else(|| format!("No default setup profile is registered for {platform}"))?;
        let config = self.upsert_profile_emulator_config(
            &profile.id,
            &profile.platform,
            exe_path,
            status,
            version,
            launch_args_template,
        )?;
        Ok(emulator_config_from_profile(config))
    }

    pub fn delete_emulator_config(&self, platform: &str) -> Result<bool, String> {
        let Some(profile) = setup_profiles::get_default_platform_setup_profile(platform) else {
            return Ok(false);
        };
        self.delete_profile_emulator_config(&profile.id)
    }

    pub fn delete_profile_emulator_config(&self, profile_id: &str) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM profile_emulator_configs WHERE profile_id = ?1",
                params![profile_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    pub fn list_profile_emulator_configs(&self) -> Result<Vec<ProfileEmulatorConfig>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template
            FROM profile_emulator_configs
            ORDER BY profile_id
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_profile_emulator_config_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_profile_emulator_config(
        &self,
        profile_id: &str,
    ) -> Result<Option<ProfileEmulatorConfig>, String> {
        self.conn
            .query_row(
                r#"
            SELECT profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template
            FROM profile_emulator_configs
            WHERE profile_id = ?1
            "#,
                params![profile_id],
                map_profile_emulator_config_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn upsert_profile_emulator_config(
        &self,
        profile_id: &str,
        platform: &str,
        exe_path: Option<&str>,
        status: &str,
        version: Option<&str>,
        launch_args_template: Option<&str>,
    ) -> Result<ProfileEmulatorConfig, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO profile_emulator_configs (
              profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(profile_id) DO UPDATE SET
              platform = excluded.platform,
              exe_path = excluded.exe_path,
              status = excluded.status,
              last_validated_at = excluded.last_validated_at,
              version = excluded.version,
              launch_args_template = excluded.launch_args_template
            "#,
                params![
                    profile_id,
                    platform,
                    exe_path,
                    status,
                    now,
                    version,
                    launch_args_template
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_profile_emulator_config(profile_id)?
            .ok_or_else(|| "Profile emulator config was not persisted.".to_string())
    }

    pub fn get_asset_installation(
        &self,
        asset_id: &str,
    ) -> Result<Option<AssetInstallation>, String> {
        self.conn
            .query_row(
                r#"
            SELECT asset_id, target_path, status, sha256, verified_at, message
            FROM asset_installations
            WHERE asset_id = ?1
            "#,
                params![asset_id],
                map_asset_installation_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_profile_system_file_imports(&self) -> Result<Vec<ProfileSystemFileImport>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT profile_id, requirement_id, target_path, status, sha256, verified_at, message
            FROM profile_system_file_imports
            ORDER BY profile_id, requirement_id
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_profile_system_file_import_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_profile_system_file_import(
        &self,
        profile_id: &str,
        requirement_id: &str,
    ) -> Result<Option<ProfileSystemFileImport>, String> {
        self.conn
            .query_row(
                r#"
            SELECT profile_id, requirement_id, target_path, status, sha256, verified_at, message
            FROM profile_system_file_imports
            WHERE profile_id = ?1 AND requirement_id = ?2
            "#,
                params![profile_id, requirement_id],
                map_profile_system_file_import_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn record_profile_system_file_import(
        &self,
        profile_id: &str,
        requirement_id: &str,
        target_path: Option<&str>,
        status: &str,
        sha256: Option<&str>,
        message: Option<&str>,
    ) -> Result<ProfileSystemFileImport, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO profile_system_file_imports (
              profile_id, requirement_id, target_path, status, sha256, verified_at, message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(profile_id, requirement_id) DO UPDATE SET
              target_path = excluded.target_path,
              status = excluded.status,
              sha256 = excluded.sha256,
              verified_at = excluded.verified_at,
              message = excluded.message
            "#,
                params![
                    profile_id,
                    requirement_id,
                    target_path,
                    status,
                    sha256,
                    now,
                    message
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_profile_system_file_import(profile_id, requirement_id)?
            .ok_or_else(|| "Profile system file import was not persisted.".to_string())
    }

    pub fn record_asset_installation(
        &self,
        asset_id: &str,
        target_path: Option<&str>,
        status: &str,
        sha256: Option<&str>,
        message: Option<&str>,
    ) -> Result<AssetInstallation, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO asset_installations (
              asset_id, target_path, status, sha256, verified_at, message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(asset_id) DO UPDATE SET
              target_path = excluded.target_path,
              status = excluded.status,
              sha256 = excluded.sha256,
              verified_at = excluded.verified_at,
              message = excluded.message
            "#,
                params![asset_id, target_path, status, sha256, now, message],
            )
            .map_err(|error| error.to_string())?;

        self.get_asset_installation(asset_id)?
            .ok_or_else(|| "Asset installation was not persisted.".to_string())
    }
}
