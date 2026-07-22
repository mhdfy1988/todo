use tauri::State;

const NORMAL_STATE_FILE: &str = "desktop-window-state.json";
const SMOKE_STATE_FILE: &str = "desktop-window-state-smoke.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeProfile {
    Normal,
    Smoke,
}

impl RuntimeProfile {
    pub(crate) fn from_args() -> Self {
        if std::env::args().any(|argument| argument == "--smoke") {
            Self::Smoke
        } else {
            Self::Normal
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Smoke => "smoke",
        }
    }

    pub(crate) fn state_file(self) -> &'static str {
        match self {
            Self::Normal => NORMAL_STATE_FILE,
            Self::Smoke => SMOKE_STATE_FILE,
        }
    }

    pub(crate) fn is_smoke(self) -> bool {
        self == Self::Smoke
    }
}

#[tauri::command]
pub(crate) fn runtime_profile(profile: State<'_, RuntimeProfile>) -> String {
    profile.as_str().to_string()
}
