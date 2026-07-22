use crate::runtime_profile::RuntimeProfile;
use serde::Serialize;
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateStatus {
    current_version: String,
    available_version: Option<String>,
}

impl UpdateStatus {
    fn current(current_version: String) -> Self {
        Self {
            current_version,
            available_version: None,
        }
    }

    fn available(current_version: String, available_version: String) -> Self {
        Self {
            current_version,
            available_version: Some(available_version),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateCommandError {
    pub code: String,
    pub message: String,
}

impl UpdateCommandError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

#[derive(Default)]
pub(crate) struct AppUpdateService {
    state: Mutex<UpdateState>,
}

#[derive(Default)]
struct UpdateState {
    activity: UpdateActivity,
    pending: Option<Update>,
}

#[derive(Default, PartialEq, Eq)]
enum UpdateActivity {
    #[default]
    Idle,
    Checking,
    Installing,
}

impl AppUpdateService {
    pub(crate) async fn check(
        &self,
        app: AppHandle,
        profile: RuntimeProfile,
    ) -> Result<UpdateStatus, UpdateCommandError> {
        ensure_normal_profile(profile)?;
        self.begin_check()?;

        let current_version = app.package_info().version.to_string();
        let exit_app = app.clone();
        let updater = app
            .updater_builder()
            .on_before_exit(move || {
                if let Err(error) = exit_app.save_window_state(StateFlags::POSITION) {
                    eprintln!("安装更新前保存窗口位置失败：{error}");
                }
                exit_app.cleanup_before_exit();
            })
            .build()
            .map_err(|error| {
                self.cancel_check();
                eprintln!("创建更新检查器失败：{error}");
                UpdateCommandError::new("UPDATE_CONFIGURATION_ERROR", "更新配置不可用")
            })?;

        let update = updater.check().await.map_err(|error| {
            self.cancel_check();
            eprintln!("检查更新失败：{error}");
            UpdateCommandError::new("UPDATE_CHECK_FAILED", "暂时无法检查更新")
        })?;
        self.complete_check(current_version, update)
    }

    pub(crate) async fn install(
        &self,
        expected_version: &str,
        profile: RuntimeProfile,
    ) -> Result<(), UpdateCommandError> {
        ensure_normal_profile(profile)?;
        let update = self.begin_install(expected_version)?;

        if let Err(error) = update.download_and_install(|_, _| {}, || {}).await {
            self.finish_install(false)?;
            eprintln!("下载或安装更新失败：{error}");
            return Err(UpdateCommandError::new(
                "UPDATE_INSTALL_FAILED",
                "更新下载或安装失败，请稍后重试",
            ));
        }

        self.finish_install(true)?;
        Ok(())
    }

    fn begin_check(&self) -> Result<(), UpdateCommandError> {
        let mut state = self.lock_state()?;
        match state.activity {
            UpdateActivity::Idle => {
                state.activity = UpdateActivity::Checking;
                Ok(())
            }
            UpdateActivity::Checking => Err(UpdateCommandError::new(
                "UPDATE_CHECKING",
                "正在检查更新，请稍候",
            )),
            UpdateActivity::Installing => Err(UpdateCommandError::new(
                "UPDATE_INSTALLING",
                "更新正在安装，请稍候",
            )),
        }
    }

    fn complete_check(
        &self,
        current_version: String,
        update: Option<Update>,
    ) -> Result<UpdateStatus, UpdateCommandError> {
        let mut state = self.lock_state()?;
        if state.activity != UpdateActivity::Checking {
            return Err(UpdateCommandError::new(
                "UPDATE_STATE_DAMAGED",
                "更新状态异常，请重启应用",
            ));
        }
        state.activity = UpdateActivity::Idle;
        match update {
            Some(update) => {
                let available_version = update.version.clone();
                state.pending = Some(update);
                Ok(UpdateStatus::available(current_version, available_version))
            }
            None => {
                state.pending = None;
                Ok(UpdateStatus::current(current_version))
            }
        }
    }

    fn cancel_check(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.activity == UpdateActivity::Checking {
                state.activity = UpdateActivity::Idle;
            }
        }
    }

    fn begin_install(&self, expected_version: &str) -> Result<Update, UpdateCommandError> {
        let mut state = self.lock_state()?;
        match state.activity {
            UpdateActivity::Checking => {
                return Err(UpdateCommandError::new(
                    "UPDATE_CHECKING",
                    "正在检查更新，请稍候",
                ));
            }
            UpdateActivity::Installing => {
                return Err(UpdateCommandError::new(
                    "UPDATE_INSTALLING",
                    "更新正在安装，请稍候",
                ));
            }
            UpdateActivity::Idle => {}
        }

        let update = state
            .pending
            .as_ref()
            .ok_or_else(|| UpdateCommandError::new("UPDATE_NOT_CHECKED", "请先重新检查更新"))?;
        if update.version != expected_version {
            return Err(UpdateCommandError::new(
                "UPDATE_VERSION_CHANGED",
                "可用版本已经变化，请重新检查更新",
            ));
        }
        let update = update.clone();
        state.activity = UpdateActivity::Installing;
        Ok(update)
    }

    fn finish_install(&self, succeeded: bool) -> Result<(), UpdateCommandError> {
        let mut state = self.lock_state()?;
        if state.activity != UpdateActivity::Installing {
            return Err(UpdateCommandError::new(
                "UPDATE_STATE_DAMAGED",
                "更新状态异常，请重启应用",
            ));
        }
        state.activity = UpdateActivity::Idle;
        if succeeded {
            state.pending = None;
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, UpdateState>, UpdateCommandError> {
        self.state.lock().map_err(|_| {
            UpdateCommandError::new("UPDATE_STATE_DAMAGED", "更新状态异常，请重启应用")
        })
    }
}

fn ensure_normal_profile(profile: RuntimeProfile) -> Result<(), UpdateCommandError> {
    if profile.is_smoke() {
        Err(UpdateCommandError::new(
            "UPDATE_DISABLED_IN_SMOKE",
            "冒烟模式不允许访问更新服务",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_profile_is_rejected_before_network_access() {
        let error = ensure_normal_profile(RuntimeProfile::Smoke).unwrap_err();
        assert_eq!(error.code, "UPDATE_DISABLED_IN_SMOKE");
    }

    #[test]
    fn update_status_uses_frontend_camel_case_contract() {
        let value = serde_json::to_value(UpdateStatus::available(
            "0.1.0".to_string(),
            "0.1.1".to_string(),
        ))
        .unwrap();
        assert_eq!(value["currentVersion"], "0.1.0");
        assert_eq!(value["availableVersion"], "0.1.1");
    }

    #[test]
    fn check_and_install_share_one_authoritative_activity_guard() {
        let service = AppUpdateService::default();
        service.begin_check().unwrap();

        assert_eq!(service.begin_check().unwrap_err().code, "UPDATE_CHECKING");
        assert_eq!(
            install_error_code(service.begin_install("0.1.1")),
            "UPDATE_CHECKING"
        );

        service.cancel_check();
        assert_eq!(
            install_error_code(service.begin_install("0.1.1")),
            "UPDATE_NOT_CHECKED"
        );
    }

    fn install_error_code(result: Result<Update, UpdateCommandError>) -> String {
        match result {
            Ok(_) => panic!("安装状态门禁意外通过"),
            Err(error) => error.code,
        }
    }
}
