use crate::runtime_profile::RuntimeProfile;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendReadyReport {
    profile: String,
    ledger_ready: bool,
}

impl FrontendReadyReport {
    pub(crate) fn is_ready_for(&self, profile: RuntimeProfile) -> bool {
        self.profile == profile.as_str() && self.ledger_ready
    }
}

#[derive(Default)]
pub(crate) struct FrontendProbeState {
    report: Mutex<Option<FrontendReadyReport>>,
    changed: Condvar,
}

impl FrontendProbeState {
    fn record(&self, report: FrontendReadyReport) -> Result<(), String> {
        let mut stored = self
            .report
            .lock()
            .map_err(|_| "前端探针状态锁已损坏".to_string())?;
        match stored.as_ref() {
            Some(existing) if existing == &report => return Ok(()),
            Some(_) => return Err("前端探针已经上报了不同结果".to_string()),
            None => {}
        }
        *stored = Some(report);
        self.changed.notify_all();
        Ok(())
    }

    pub(crate) fn wait_for_report(&self, timeout: Duration) -> Result<FrontendReadyReport, String> {
        let stored = self
            .report
            .lock()
            .map_err(|_| "前端探针状态锁已损坏".to_string())?;
        let (stored, wait_result) = self
            .changed
            .wait_timeout_while(stored, timeout, |report| report.is_none())
            .map_err(|_| "等待前端探针时状态锁已损坏".to_string())?;
        if wait_result.timed_out() && stored.is_none() {
            return Err(format!("等待前端就绪报告超时（{} 秒）", timeout.as_secs()));
        }
        stored
            .clone()
            .ok_or_else(|| "前端探针被唤醒但没有报告".to_string())
    }
}

#[tauri::command]
pub(crate) fn report_frontend_ready(
    report: FrontendReadyReport,
    profile: State<'_, RuntimeProfile>,
    probe: State<'_, FrontendProbeState>,
) -> Result<(), String> {
    if report.profile != profile.as_str() {
        return Err(format!(
            "前端探针运行档案不一致：前端={}，后端={}",
            report.profile,
            profile.as_str()
        ));
    }
    probe.record(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_probe_is_idempotent_for_the_same_report() {
        let state = FrontendProbeState::default();
        let report = FrontendReadyReport {
            profile: "smoke".to_string(),
            ledger_ready: true,
        };
        state.record(report.clone()).expect("首次上报应成功");
        state.record(report.clone()).expect("相同上报应幂等");
        assert_eq!(
            state
                .wait_for_report(Duration::from_millis(1))
                .expect("应立即读到报告"),
            report
        );
    }

    #[test]
    fn frontend_probe_rejects_conflicting_reports() {
        let state = FrontendProbeState::default();
        state
            .record(FrontendReadyReport {
                profile: "smoke".to_string(),
                ledger_ready: true,
            })
            .expect("首次上报应成功");
        let error = state
            .record(FrontendReadyReport {
                profile: "smoke".to_string(),
                ledger_ready: false,
            })
            .expect_err("不同上报必须拒绝");
        assert!(error.contains("不同结果"));
    }
}
