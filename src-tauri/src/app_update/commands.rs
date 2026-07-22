use super::service::{AppUpdateService, UpdateCommandError, UpdateStatus};
use crate::runtime_profile::RuntimeProfile;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) async fn check_for_update(
    app: AppHandle,
    profile: State<'_, RuntimeProfile>,
    updates: State<'_, AppUpdateService>,
) -> Result<UpdateStatus, UpdateCommandError> {
    updates.check(app, *profile).await
}

#[tauri::command]
pub(crate) async fn install_update(
    expected_version: String,
    profile: State<'_, RuntimeProfile>,
    updates: State<'_, AppUpdateService>,
) -> Result<(), UpdateCommandError> {
    updates.install(&expected_version, *profile).await
}
