use crate::window_geometry::{Point, Size, WorkArea};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowMode {
    Expanded,
    Capsule,
    Pet,
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowSpec {
    pub(crate) logical_width: f64,
    pub(crate) logical_height: f64,
    pub(crate) allows_focus: bool,
    pub(crate) dock_to_edge: bool,
}

impl WindowMode {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "expanded" => Ok(Self::Expanded),
            "capsule" => Ok(Self::Capsule),
            "pet" => Ok(Self::Pet),
            "edge" => Ok(Self::Edge),
            _ => Err(format!("不支持的窗口状态：{value}")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Capsule => "capsule",
            Self::Pet => "pet",
            Self::Edge => "edge",
        }
    }

    pub(crate) fn spec(self) -> WindowSpec {
        match self {
            Self::Expanded => WindowSpec {
                logical_width: 420.0,
                logical_height: 520.0,
                allows_focus: true,
                dock_to_edge: false,
            },
            Self::Capsule => WindowSpec {
                logical_width: 310.0,
                logical_height: 92.0,
                allows_focus: false,
                dock_to_edge: false,
            },
            Self::Pet => WindowSpec {
                logical_width: 104.0,
                logical_height: 104.0,
                allows_focus: false,
                dock_to_edge: false,
            },
            Self::Edge => WindowSpec {
                logical_width: 72.0,
                logical_height: 104.0,
                allows_focus: false,
                dock_to_edge: true,
            },
        }
    }
}

pub(crate) struct DesktopState {
    mode: Mutex<WindowMode>,
    focusable: AtomicBool,
    tray_ready: AtomicBool,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            mode: Mutex::new(WindowMode::Capsule),
            focusable: AtomicBool::new(false),
            tray_ready: AtomicBool::new(false),
        }
    }
}

impl DesktopState {
    pub(crate) fn mode(&self) -> Result<WindowMode, String> {
        self.mode
            .lock()
            .map(|mode| *mode)
            .map_err(|_| "窗口状态锁已损坏".to_string())
    }

    pub(crate) fn set_mode(&self, mode: WindowMode) -> Result<(), String> {
        *self
            .mode
            .lock()
            .map_err(|_| "窗口状态锁已损坏".to_string())? = mode;
        Ok(())
    }

    pub(crate) fn focusable(&self) -> bool {
        self.focusable.load(Ordering::SeqCst)
    }

    pub(crate) fn set_focusable(&self, enabled: bool) {
        self.focusable.store(enabled, Ordering::SeqCst);
    }

    pub(crate) fn tray_ready(&self) -> bool {
        self.tray_ready.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_tray_ready(&self) {
        self.tray_ready.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowStatus {
    pub(crate) mode: String,
    pub(crate) visible: bool,
    pub(crate) focused: bool,
    pub(crate) focusable: bool,
    pub(crate) always_on_top: bool,
    pub(crate) tray_ready: bool,
    pub(crate) in_work_area: bool,
    pub(crate) position: Point,
    pub(crate) size: Size,
    pub(crate) work_area: Option<WorkArea>,
    pub(crate) monitor_name: Option<String>,
    pub(crate) scale_factor: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_specs_keep_the_existing_protocol_and_behavior() {
        let cases = [
            (WindowMode::Expanded, "expanded", 420.0, 520.0, true, false),
            (WindowMode::Capsule, "capsule", 310.0, 92.0, false, false),
            (WindowMode::Pet, "pet", 104.0, 104.0, false, false),
            (WindowMode::Edge, "edge", 72.0, 104.0, false, true),
        ];

        for (mode, wire_value, width, height, allows_focus, dock_to_edge) in cases {
            assert_eq!(WindowMode::parse(wire_value), Ok(mode));
            assert_eq!(mode.as_str(), wire_value);
            assert_eq!(
                mode.spec(),
                WindowSpec {
                    logical_width: width,
                    logical_height: height,
                    allows_focus,
                    dock_to_edge,
                }
            );
        }
    }

    #[test]
    fn tauri_main_window_matches_the_capsule_contract() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
            .expect("Tauri 配置必须是合法 JSON");
        let main_window = config["app"]["windows"]
            .as_array()
            .and_then(|windows| {
                windows
                    .iter()
                    .find(|window| window["label"] == crate::desktop::MAIN_WINDOW)
            })
            .expect("Tauri 配置必须包含主悬浮窗");
        let capsule = WindowMode::Capsule.spec();

        assert_eq!(main_window["width"].as_f64(), Some(capsule.logical_width));
        assert_eq!(main_window["height"].as_f64(), Some(capsule.logical_height));
        assert_eq!(main_window["focus"].as_bool(), Some(false));
        assert_eq!(main_window["focusable"].as_bool(), Some(false));
    }
}
