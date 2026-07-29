use std::{fmt, hash::Hash};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    #[must_use]
    pub const fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn right(self) -> i32 {
        self.left.saturating_add(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.top.saturating_add(self.height)
    }

    #[must_use]
    pub fn area(self) -> i64 {
        i64::from(self.width.max(0)) * i64::from(self.height.max(0))
    }

    #[must_use]
    pub fn aspect_ratio(self) -> Option<f64> {
        (self.width > 0 && self.height > 0).then(|| f64::from(self.width) / f64::from(self.height))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl Size {
    #[must_use]
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WindowId(pub u64);

impl fmt::Display for WindowId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "window-{:X}", self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct WindowSignature {
    pub process_name: String,
    pub class_name: String,
    pub title_pattern: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDisposition {
    Table,
    Parked,
    TopRight,
    FreeSpace,
    Ignored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMode {
    Arranged,
    Parked,
    TopRight,
    FreeSpace,
    Ignored,
}

#[derive(Clone, Debug)]
pub struct WindowCandidate {
    pub id: WindowId,
    pub label: String,
    pub process_name: String,
    pub class_name: String,
    pub signature: WindowSignature,
    pub rect: Rect,
    pub is_clubgg: bool,
    pub likely_table: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableStatus {
    Ready,
    Parked,
    AccessDenied,
    MoveFailed(String),
}

impl fmt::Display for TableStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => formatter.write_str("Ready"),
            Self::Parked => formatter.write_str("Parked"),
            Self::AccessDenied => formatter.write_str("Access denied"),
            Self::MoveFailed(message) => write!(formatter, "Move failed: {message}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ManagedTable {
    pub id: WindowId,
    pub label: String,
    pub signature: WindowSignature,
    pub enabled: bool,
    pub last_active_rect: Rect,
    pub status: TableStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonitorInfo {
    pub id: String,
    pub label: String,
    pub work_area: Rect,
    pub primary: bool,
}

#[derive(Clone, Debug)]
pub struct CandidateView {
    pub id: WindowId,
    pub label: String,
    pub process_name: String,
    pub class_name: String,
    pub is_clubgg: bool,
    pub likely_table: bool,
    pub mode: WindowMode,
    pub slot: Option<usize>,
    pub status: Option<TableStatus>,
}

#[derive(Clone, Debug)]
pub struct UiSnapshot {
    pub tables: Vec<ManagedTable>,
    pub candidates: Vec<CandidateView>,
    pub monitors: Vec<MonitorInfo>,
    pub selected_monitor: Option<String>,
    pub auto_arrange: bool,
    pub aspect_ratio: f64,
    pub status_message: String,
    pub hotkeys: crate::config::HotkeySettings,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            tables: Vec::new(),
            candidates: Vec::new(),
            monitors: Vec::new(),
            selected_monitor: None,
            auto_arrange: true,
            aspect_ratio: 4.0 / 3.0,
            status_message: "Starting…".to_owned(),
            hotkeys: crate::config::HotkeySettings::default(),
        }
    }
}

#[derive(Clone, Debug, Error)]
pub enum BackendError {
    #[error("access denied")]
    AccessDenied,
    #[error("window no longer exists")]
    WindowGone,
    #[error("{0}")]
    Other(String),
}

pub trait WindowBackend: Send + Sync + 'static {
    fn enumerate_candidates(&self) -> Result<Vec<WindowCandidate>, BackendError>;
    fn monitors(&self) -> Result<Vec<MonitorInfo>, BackendError>;
    fn move_resize(&self, id: WindowId, rect: Rect) -> Result<Rect, BackendError>;
    fn minimum_size(&self, id: WindowId, aspect_ratio: f64) -> Result<Size, BackendError>;
    fn highlight(&self, id: WindowId) -> Result<(), BackendError>;
    fn foreground_window(&self) -> Option<WindowId>;
}
