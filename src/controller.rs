use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use log::{info, warn};

use crate::{
    config::{AppConfig, ApplicationDefault, ConfigStore, HotkeySettings},
    layout::{DEFAULT_ASPECT_RATIO, calculate_layout, right_side_free_rect},
    model::{
        BackendError, CandidateDisposition, CandidateView, ManagedTable, MonitorInfo, Rect,
        TableStatus, UiSnapshot, WindowBackend, WindowCandidate, WindowId, WindowMode,
    },
};

const FALLBACK_RECONCILE_INTERVAL: Duration = Duration::from_secs(10);
const DISCOVERY_DEBOUNCE: Duration = Duration::from_millis(200);
const REFLOW_DEBOUNCE: Duration = Duration::from_millis(500);
const COMMAND_QUEUE_CAPACITY: usize = 64;
const SNAPSHOT_QUEUE_CAPACITY: usize = 1;
const CONTROLLER_STACK_BYTES: usize = 512 * 1024;

pub type UiWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug)]
pub enum ControllerCommand {
    ForceArrange,
    NativeWindowEvent(&'static AtomicBool),
    SetAutoArrange(bool),
    SetReserveTwoSlots(bool),
    SetDefaultApplicationMode(ApplicationDefault),
    SetEnabled { id: WindowId, enabled: bool },
    ToggleFocused,
    ToggleSlot(usize),
    Reorder { from: usize, to: usize },
    SelectMonitor(String),
    SetWindowMode { id: WindowId, mode: WindowMode },
    Locate(WindowId),
    SetHotkeys(HotkeySettings),
    Shutdown,
}

pub struct ControllerHandle {
    pub commands: Sender<ControllerCommand>,
    pub snapshots: Receiver<Arc<UiSnapshot>>,
}

#[must_use]
pub fn spawn_controller(
    backend: Arc<dyn WindowBackend>,
    config: AppConfig,
    store: ConfigStore,
) -> ControllerHandle {
    spawn_controller_with_waker(backend, config, store, Arc::new(|| {}))
}

#[must_use]
pub fn spawn_controller_with_waker(
    backend: Arc<dyn WindowBackend>,
    config: AppConfig,
    store: ConfigStore,
    wake_ui: UiWake,
) -> ControllerHandle {
    let (command_tx, command_rx) = bounded(COMMAND_QUEUE_CAPACITY);
    let (snapshot_tx, snapshot_rx) = bounded(SNAPSHOT_QUEUE_CAPACITY);
    let stale_snapshot_rx = snapshot_rx.clone();
    thread::Builder::new()
        .name("clubgg-controller".to_owned())
        .stack_size(CONTROLLER_STACK_BYTES)
        .spawn(move || {
            Controller::new(
                backend,
                config,
                store,
                snapshot_tx,
                stale_snapshot_rx,
                wake_ui,
            )
            .run(command_rx);
        })
        .expect("controller thread must start");
    ControllerHandle {
        commands: command_tx,
        snapshots: snapshot_rx,
    }
}

struct Controller {
    backend: Arc<dyn WindowBackend>,
    config: AppConfig,
    store: ConfigStore,
    snapshot_tx: Sender<Arc<UiSnapshot>>,
    stale_snapshot_rx: Receiver<Arc<UiSnapshot>>,
    wake_ui: UiWake,
    last_published: Option<Arc<UiSnapshot>>,
    tables: Vec<ManagedTable>,
    candidates: Vec<WindowCandidate>,
    window_statuses: HashMap<WindowId, TableStatus>,
    monitors: Vec<MonitorInfo>,
    dirty_since: Option<Instant>,
    last_reconcile: Instant,
    discovery_due: Option<Instant>,
    status_message: String,
}

impl Controller {
    fn new(
        backend: Arc<dyn WindowBackend>,
        config: AppConfig,
        store: ConfigStore,
        snapshot_tx: Sender<Arc<UiSnapshot>>,
        stale_snapshot_rx: Receiver<Arc<UiSnapshot>>,
        wake_ui: UiWake,
    ) -> Self {
        Self {
            backend,
            config,
            store,
            snapshot_tx,
            stale_snapshot_rx,
            wake_ui,
            last_published: None,
            tables: Vec::new(),
            candidates: Vec::new(),
            window_statuses: HashMap::new(),
            monitors: Vec::new(),
            dirty_since: None,
            last_reconcile: Instant::now() - FALLBACK_RECONCILE_INTERVAL,
            discovery_due: None,
            status_message: "Looking for ClubGG tables…".to_owned(),
        }
    }

    fn run(mut self, commands: Receiver<ControllerCommand>) {
        self.reconcile();
        self.publish();

        loop {
            match commands.recv_timeout(self.next_wait()) {
                Ok(ControllerCommand::Shutdown) => break,
                Ok(command) => self.handle(command),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }

            for command in commands.try_iter().take(COMMAND_QUEUE_CAPACITY) {
                if matches!(command, ControllerCommand::Shutdown) {
                    return;
                }
                self.handle(command);
            }

            let discovery_due = self.discovery_due.is_some_and(|due| Instant::now() >= due);
            if discovery_due || self.last_reconcile.elapsed() >= FALLBACK_RECONCILE_INTERVAL {
                self.discovery_due = None;
                self.reconcile();
            }

            if self.config.auto_arrange
                && self
                    .dirty_since
                    .is_some_and(|dirty| dirty.elapsed() >= REFLOW_DEBOUNCE)
            {
                self.arrange();
            }
        }
    }

    fn next_wait(&self) -> Duration {
        let mut wait = FALLBACK_RECONCILE_INTERVAL.saturating_sub(self.last_reconcile.elapsed());
        if let Some(due) = self.discovery_due {
            wait = wait.min(due.saturating_duration_since(Instant::now()));
        }
        if let Some(dirty) = self.dirty_since {
            wait = wait.min(REFLOW_DEBOUNCE.saturating_sub(dirty.elapsed()));
        }
        wait
    }

    fn handle(&mut self, command: ControllerCommand) {
        match command {
            ControllerCommand::ForceArrange => {
                self.discovery_due = None;
                self.reconcile();
                self.arrange();
            }
            ControllerCommand::NativeWindowEvent(pending) => {
                pending.store(false, Ordering::Release);
                self.discovery_due = Some(Instant::now() + DISCOVERY_DEBOUNCE);
            }
            ControllerCommand::SetAutoArrange(enabled) => {
                self.config.auto_arrange = enabled;
                self.save_config();
                if enabled {
                    self.discovery_due = None;
                    self.reconcile();
                    self.arrange();
                } else {
                    self.publish();
                }
            }
            ControllerCommand::SetReserveTwoSlots(enabled) => {
                self.config.reserve_two_slots = enabled;
                self.save_config();
                self.arrange();
            }
            ControllerCommand::SetDefaultApplicationMode(mode) => {
                self.config.default_application_mode = mode;
                self.save_config();
                self.reconcile();
                self.arrange();
            }
            ControllerCommand::SetEnabled { id, enabled } => {
                self.set_enabled(id, enabled);
            }
            ControllerCommand::ToggleFocused => {
                if let Some(id) = self.backend.foreground_window()
                    && let Some(index) = self.tables.iter().position(|table| table.id == id)
                {
                    let enabled = !self.tables[index].enabled;
                    self.set_enabled(id, enabled);
                }
            }
            ControllerCommand::ToggleSlot(slot) => {
                if let Some(table) = self.tables.get(slot).cloned() {
                    self.set_enabled(table.id, !table.enabled);
                }
            }
            ControllerCommand::Reorder { from, to } => {
                if from < self.tables.len() && to < self.tables.len() && from != to {
                    let table = self.tables.remove(from);
                    self.tables.insert(to, table);
                    self.persist_table_order();
                    self.arrange();
                }
            }
            ControllerCommand::SelectMonitor(id) => {
                self.config.selected_monitor = Some(id);
                self.save_config();
                self.arrange();
            }
            ControllerCommand::SetWindowMode { id, mode } => self.set_window_mode(id, mode),
            ControllerCommand::Locate(id) => {
                if let Err(error) = self.backend.locate(id) {
                    self.status_message = format!("Could not locate window: {error}");
                    self.publish();
                }
            }
            ControllerCommand::SetHotkeys(hotkeys) => {
                self.config.hotkeys = hotkeys;
                self.save_config();
                self.publish();
            }
            ControllerCommand::Shutdown => {}
        }
    }

    fn set_enabled(&mut self, id: WindowId, enabled: bool) {
        let Some(index) = self.tables.iter().position(|table| table.id == id) else {
            return;
        };
        self.tables[index].enabled = enabled;
        let disposition = if enabled {
            CandidateDisposition::Table
        } else {
            CandidateDisposition::Parked
        };
        self.config
            .set_disposition(self.tables[index].signature.clone(), disposition);
        self.save_config();
        if enabled {
            self.tables[index].status = TableStatus::Ready;
        }
        self.arrange();
    }

    fn set_window_mode(&mut self, id: WindowId, mode: WindowMode) {
        let Some((signature, is_clubgg)) = self
            .candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| (candidate.signature.clone(), candidate.is_clubgg))
        else {
            return;
        };
        let disposition = match mode {
            WindowMode::Arranged => CandidateDisposition::Table,
            WindowMode::Parked => CandidateDisposition::Parked,
            WindowMode::TopRight => CandidateDisposition::TopRight,
            WindowMode::FreeSpace => CandidateDisposition::FreeSpace,
            WindowMode::Ignored => CandidateDisposition::Ignored,
        };
        if is_clubgg {
            self.config.set_disposition(signature, disposition);
        } else {
            self.config
                .set_application_disposition(signature, disposition);
        }
        if mode == WindowMode::Ignored {
            self.window_statuses.remove(&id);
        }
        self.save_config();
        self.reconcile();

        if let Some(index) = self.tables.iter().position(|table| table.id == id) {
            match mode {
                WindowMode::Arranged => {
                    self.tables[index].enabled = true;
                    self.tables[index].status = TableStatus::Ready;
                }
                WindowMode::Parked => {
                    self.tables[index].enabled = false;
                }
                WindowMode::TopRight | WindowMode::FreeSpace | WindowMode::Ignored => {}
            }
        }
        self.arrange();
    }

    fn reconcile(&mut self) {
        self.last_reconcile = Instant::now();
        let mut candidates = match self.backend.enumerate_candidates() {
            Ok(candidates) => candidates,
            Err(error) => {
                self.status_message = format!("Window discovery failed: {error}");
                self.publish();
                return;
            }
        };
        let previous_order: HashMap<_, _> = self
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.id, index))
            .collect();
        let discovered_order: HashMap<_, _> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.id, index))
            .collect();
        candidates.sort_by_key(|candidate| {
            previous_order.get(&candidate.id).map_or_else(
                || (1_u8, discovered_order[&candidate.id]),
                |index| (0_u8, *index),
            )
        });
        let monitors = match self.backend.monitors() {
            Ok(monitors) => monitors,
            Err(error) => {
                self.status_message = format!("Monitor discovery failed: {error}");
                Vec::new()
            }
        };

        let old_ids: HashSet<_> = self.tables.iter().map(|table| table.id).collect();
        let old_positioned_ids = self.positioned_window_ids();
        let candidate_by_id: HashMap<_, _> =
            candidates.iter().map(|item| (item.id, item)).collect();

        self.tables
            .retain(|table| candidate_by_id.contains_key(&table.id));
        for table in &mut self.tables {
            if let Some(candidate) = candidate_by_id.get(&table.id) {
                table.label.clone_from(&candidate.label);
                table.signature.clone_from(&candidate.signature);
                if table.enabled {
                    table.last_active_rect = candidate.rect;
                }
            }
        }

        let mut table_order_changed = false;
        for candidate in &candidates {
            let disposition = self.disposition_for_candidate(candidate);
            let should_manage = match disposition {
                Some(CandidateDisposition::Table | CandidateDisposition::Parked) => true,
                Some(
                    CandidateDisposition::TopRight
                    | CandidateDisposition::FreeSpace
                    | CandidateDisposition::Ignored,
                ) => false,
                None => candidate.likely_table,
            };
            let existing = self
                .tables
                .iter()
                .position(|table| table.id == candidate.id);

            if should_manage && existing.is_none() {
                let enabled = disposition != Some(CandidateDisposition::Parked);
                if !self.config.table_order.contains(&candidate.signature) {
                    self.config.table_order.push(candidate.signature.clone());
                    table_order_changed = true;
                }
                self.tables.push(ManagedTable {
                    id: candidate.id,
                    label: candidate.label.clone(),
                    signature: candidate.signature.clone(),
                    enabled,
                    last_active_rect: candidate.rect,
                    status: TableStatus::Ready,
                });
            } else if !should_manage && let Some(index) = existing {
                self.tables.remove(index);
            }
        }
        let stored_order: HashMap<_, _> = self
            .config
            .table_order
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, signature)| (signature, index))
            .collect();
        self.tables.sort_by_key(|table| {
            stored_order
                .get(&table.signature)
                .map_or((1_u8, usize::MAX), |index| (0_u8, *index))
        });
        if table_order_changed {
            self.save_config();
        }

        let new_ids: HashSet<_> = self.tables.iter().map(|table| table.id).collect();
        let table_set_changed = old_ids != new_ids;
        let monitor_changed = self.monitors != monitors;
        self.candidates = candidates;
        self.monitors = monitors;
        self.window_statuses
            .retain(|id, _| self.candidates.iter().any(|candidate| candidate.id == *id));
        let positioned_set_changed = old_positioned_ids != self.positioned_window_ids();

        if self.config.table_aspect_ratio.is_none() {
            self.config.table_aspect_ratio = self
                .tables
                .iter()
                .filter_map(|table| table.last_active_rect.aspect_ratio())
                .find(|ratio| (0.8..=2.5).contains(ratio));
            if self.config.table_aspect_ratio.is_some() {
                self.save_config();
            }
        }

        if self.selected_monitor().is_none()
            && let Some(primary) = self
                .monitors
                .iter()
                .find(|monitor| monitor.primary)
                .or_else(|| self.monitors.first())
        {
            self.config.selected_monitor = Some(primary.id.clone());
            self.save_config();
        }

        if table_set_changed
            || monitor_changed
            || self
                .tables
                .iter()
                .any(|table| !table.enabled && table.status != TableStatus::Parked)
        {
            self.park_all();
        }

        if table_set_changed || positioned_set_changed || monitor_changed {
            info!(
                "discovery state changed; candidates={}; managed_tables={}; positioned_windows={}; monitors={}",
                self.candidates.len(),
                self.tables.len(),
                self.positioned_window_ids().len(),
                self.monitors.len()
            );
            self.mark_dirty();
        }
        let poker_window_count = self
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.likely_table || self.tables.iter().any(|table| table.id == candidate.id)
            })
            .count();
        let other_window_count = self.candidates.len().saturating_sub(poker_window_count);
        self.status_message = format!(
            "{} poker table{} · {} other window{} discovered",
            poker_window_count,
            if poker_window_count == 1 { "" } else { "s" },
            other_window_count,
            if other_window_count == 1 { "" } else { "s" }
        );
        self.publish();
    }

    fn positioned_window_ids(&self) -> HashSet<WindowId> {
        self.candidates
            .iter()
            .filter_map(|candidate| {
                matches!(
                    self.disposition_for_candidate(candidate),
                    Some(CandidateDisposition::TopRight | CandidateDisposition::FreeSpace)
                )
                .then_some(candidate.id)
            })
            .collect()
    }

    fn disposition_for_candidate(
        &self,
        candidate: &WindowCandidate,
    ) -> Option<CandidateDisposition> {
        self.config
            .disposition_for(&candidate.signature)
            .or_else(|| {
                (!candidate.is_clubgg).then(|| self.config.default_application_mode.disposition())
            })
    }

    fn selected_monitor(&self) -> Option<&MonitorInfo> {
        self.config
            .selected_monitor
            .as_ref()
            .and_then(|id| self.monitors.iter().find(|monitor| &monitor.id == id))
            .or_else(|| self.monitors.iter().find(|monitor| monitor.primary))
            .or_else(|| self.monitors.first())
    }

    fn arrange(&mut self) {
        self.dirty_since = None;
        let Some(monitor) = self.selected_monitor().cloned() else {
            self.status_message = "No usable monitor found.".to_owned();
            self.publish();
            return;
        };
        let enabled_indices: Vec<_> = self
            .tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| table.enabled.then_some(index))
            .collect();
        let ratio = self
            .config
            .table_aspect_ratio
            .unwrap_or(DEFAULT_ASPECT_RATIO);
        let layout = calculate_layout(monitor.work_area, enabled_indices.len(), ratio);
        info!(
            "arrangement started; active_tables={}; columns={}; rows={}",
            enabled_indices.len(),
            layout.columns,
            layout.rows
        );

        let mut actual_sizes = Vec::new();
        let mut moved_count = 0_usize;
        let mut failed_count = 0_usize;
        let mut access_denied_count = 0_usize;
        for (slot, index) in enabled_indices.iter().copied().enumerate() {
            let requested = layout.rectangles[slot];
            match self.backend.move_resize(self.tables[index].id, requested) {
                Ok(actual) => {
                    self.tables[index].last_active_rect = actual;
                    self.tables[index].status = TableStatus::Ready;
                    actual_sizes.push((actual.width, actual.height));
                    moved_count += 1;
                }
                Err(BackendError::AccessDenied) => {
                    self.tables[index].status = TableStatus::AccessDenied;
                    failed_count += 1;
                    access_denied_count += 1;
                    warn!("table move was denied by Windows; slot={}", slot + 1);
                }
                Err(BackendError::WindowGone) => {
                    failed_count += 1;
                    warn!("table closed before it could be moved; slot={}", slot + 1);
                }
                Err(BackendError::Other(message)) => {
                    warn!("table move failed; slot={}; error={message}", slot + 1);
                    self.tables[index].status = TableStatus::MoveFailed(message);
                    failed_count += 1;
                }
            }
        }

        if let Some((common_width, common_height)) = actual_sizes
            .iter()
            .copied()
            .reduce(|left, right| (left.0.min(right.0), left.1.min(right.1)))
            && actual_sizes
                .iter()
                .any(|size| *size != (common_width, common_height))
        {
            for (slot, index) in enabled_indices.iter().copied().enumerate() {
                let requested = layout.rectangles[slot];
                let column = requested
                    .left
                    .saturating_sub(monitor.work_area.left)
                    .div_euclid(layout.table_width);
                let row = requested
                    .top
                    .saturating_sub(monitor.work_area.top)
                    .div_euclid(layout.table_height);
                let rect = Rect::new(
                    monitor.work_area.left + column.saturating_mul(common_width),
                    monitor.work_area.top + row.saturating_mul(common_height),
                    common_width,
                    common_height,
                );
                if let Ok(actual) = self.backend.move_resize(self.tables[index].id, rect) {
                    self.tables[index].last_active_rect = actual;
                }
            }
        }

        self.park_all();

        let (positioned_requested, positioned_moved, positioned_failed, positioned_denied) =
            self.position_other_windows(&monitor);
        failed_count += positioned_failed;
        access_denied_count += positioned_denied;
        let requested_count = enabled_indices.len() + positioned_requested;
        let total_moved = moved_count + positioned_moved;

        self.status_message = if requested_count == 0 {
            "No windows selected for positioning.".to_owned()
        } else if failed_count == 0 && enabled_indices.is_empty() {
            format!(
                "Positioned {positioned_moved} application window{} on {}",
                if positioned_moved == 1 { "" } else { "s" },
                monitor.label
            )
        } else if failed_count == 0 && positioned_requested == 0 {
            format!(
                "Arranged {moved_count} active table{} on {}",
                if moved_count == 1 { "" } else { "s" },
                monitor.label
            )
        } else if failed_count == 0 {
            format!(
                "Arranged {moved_count} table{} and positioned {positioned_moved} application window{} on {}",
                if moved_count == 1 { "" } else { "s" },
                if positioned_moved == 1 { "" } else { "s" },
                monitor.label
            )
        } else if access_denied_count > 0 {
            format!(
                "Moved {total_moved} of {requested_count} selected windows; {access_denied_count} access denied. Run both apps at the same privilege level."
            )
        } else {
            format!(
                "Moved {total_moved} of {requested_count} selected windows; {failed_count} failed. See window status and log."
            )
        };
        info!(
            "arrangement finished; active_tables={}; positioned_windows={}; moved={}; failed={}",
            enabled_indices.len(),
            positioned_requested,
            total_moved,
            failed_count
        );
        self.publish();
    }

    fn position_other_windows(&mut self, monitor: &MonitorInfo) -> (usize, usize, usize, usize) {
        let mut occupied: Vec<_> = self
            .tables
            .iter()
            .filter(|table| table.enabled)
            .map(|table| table.last_active_rect)
            .collect();
        if self.config.reserve_two_slots && occupied.len() < 2 {
            let ratio = self
                .config
                .table_aspect_ratio
                .unwrap_or(DEFAULT_ASPECT_RATIO);
            occupied.extend(calculate_layout(monitor.work_area, 2, ratio).rectangles);
        }
        let free_rect = right_side_free_rect(monitor.work_area, &occupied);
        let selected: Vec<_> = self
            .candidates
            .iter()
            .filter_map(|candidate| {
                let mode = match self.disposition_for_candidate(candidate) {
                    Some(CandidateDisposition::TopRight) => WindowMode::TopRight,
                    Some(CandidateDisposition::FreeSpace) => WindowMode::FreeSpace,
                    _ => return None,
                };
                Some((candidate.id, candidate.rect, mode))
            })
            .collect();

        let mut moved = 0_usize;
        let mut failed = 0_usize;
        let mut access_denied = 0_usize;
        for (id, current_rect, mode) in &selected {
            let requested = match mode {
                WindowMode::TopRight => {
                    let width = current_rect.width.clamp(1, monitor.work_area.width);
                    let height = current_rect.height.clamp(1, monitor.work_area.height);
                    Rect::new(
                        monitor.work_area.right().saturating_sub(width),
                        monitor.work_area.top,
                        width,
                        height,
                    )
                }
                WindowMode::FreeSpace => {
                    let current_ratio = current_rect.aspect_ratio().unwrap_or(DEFAULT_ASPECT_RATIO);
                    let minimum = self
                        .backend
                        .minimum_size(*id, current_ratio)
                        .unwrap_or_else(|_| crate::model::Size::new(240, 180));
                    if free_rect.width < minimum.width || free_rect.height < minimum.height {
                        self.window_statuses.insert(
                            *id,
                            TableStatus::MoveFailed(
                                "No usable space remains to the right of active poker tables"
                                    .to_owned(),
                            ),
                        );
                        failed += 1;
                        continue;
                    }
                    free_rect
                }
                WindowMode::Arranged | WindowMode::Parked | WindowMode::Ignored => continue,
            };

            match self.backend.move_resize(*id, requested) {
                Ok(_) => {
                    self.window_statuses.insert(*id, TableStatus::Ready);
                    moved += 1;
                }
                Err(BackendError::AccessDenied) => {
                    self.window_statuses.insert(*id, TableStatus::AccessDenied);
                    failed += 1;
                    access_denied += 1;
                }
                Err(error) => {
                    self.window_statuses
                        .insert(*id, TableStatus::MoveFailed(error.to_string()));
                    failed += 1;
                }
            }
        }
        (selected.len(), moved, failed, access_denied)
    }

    fn park_all(&mut self) {
        let Some(monitor) = self.selected_monitor().cloned() else {
            return;
        };
        let ratio = self
            .config
            .table_aspect_ratio
            .unwrap_or(DEFAULT_ASPECT_RATIO);
        let parked: Vec<_> = self
            .tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| (!table.enabled).then_some(index))
            .collect();
        let mut cursor_right = monitor.work_area.right();
        let mut cursor_bottom = monitor.work_area.bottom();
        let mut row_height = 0_i32;

        for index in parked {
            let size = self
                .backend
                .minimum_size(self.tables[index].id, ratio)
                .unwrap_or_else(|_| crate::model::Size::new(240, 180));
            let width = size.width.clamp(1, monitor.work_area.width);
            let height = size.height.clamp(1, monitor.work_area.height);
            if cursor_right.saturating_sub(width) < monitor.work_area.left {
                cursor_right = monitor.work_area.right();
                cursor_bottom = cursor_bottom.saturating_sub(row_height);
                row_height = 0;
            }
            let rect = Rect::new(
                cursor_right.saturating_sub(width),
                cursor_bottom
                    .saturating_sub(height)
                    .max(monitor.work_area.top),
                width,
                height,
            );
            match self.backend.move_resize(self.tables[index].id, rect) {
                Ok(actual) => {
                    self.tables[index].status = TableStatus::Parked;
                    cursor_right = actual.left;
                    row_height = row_height.max(actual.height);
                }
                Err(BackendError::AccessDenied) => {
                    self.tables[index].status = TableStatus::AccessDenied;
                    cursor_right = rect.left;
                    row_height = row_height.max(height);
                }
                Err(error) => {
                    self.tables[index].status = TableStatus::MoveFailed(error.to_string());
                    cursor_right = rect.left;
                    row_height = row_height.max(height);
                }
            }
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn persist_table_order(&mut self) {
        let visible: HashSet<_> = self
            .tables
            .iter()
            .map(|table| table.signature.clone())
            .collect();
        let mut order: Vec<_> = self
            .tables
            .iter()
            .map(|table| table.signature.clone())
            .collect();
        order.extend(
            self.config
                .table_order
                .iter()
                .filter(|signature| !visible.contains(*signature))
                .cloned(),
        );
        self.config.table_order = order;
        self.save_config();
    }

    fn save_config(&mut self) {
        if let Err(error) = self.store.save(&self.config) {
            warn!("could not save configuration: {error}");
            self.status_message = format!("Settings could not be saved: {error}");
        }
    }

    fn publish(&mut self) {
        let candidates = self
            .candidates
            .iter()
            .map(|candidate| CandidateView {
                id: candidate.id,
                label: candidate.label.clone(),
                process_name: candidate.process_name.clone(),
                class_name: candidate.class_name.clone(),
                is_clubgg: candidate.is_clubgg,
                likely_table: candidate.likely_table,
                mode: self
                    .tables
                    .iter()
                    .find(|table| table.id == candidate.id)
                    .map_or_else(
                        || match self.disposition_for_candidate(candidate) {
                            Some(CandidateDisposition::TopRight) => WindowMode::TopRight,
                            Some(CandidateDisposition::FreeSpace) => WindowMode::FreeSpace,
                            _ => WindowMode::Ignored,
                        },
                        |table| {
                            if table.enabled {
                                WindowMode::Arranged
                            } else {
                                WindowMode::Parked
                            }
                        },
                    ),
                slot: self
                    .tables
                    .iter()
                    .position(|table| table.id == candidate.id)
                    .map(|index| index + 1),
                status: self
                    .tables
                    .iter()
                    .find(|table| table.id == candidate.id)
                    .map(|table| table.status.clone())
                    .or_else(|| self.window_statuses.get(&candidate.id).cloned()),
            })
            .collect();
        let snapshot = UiSnapshot {
            tables: self.tables.clone(),
            candidates,
            monitors: self.monitors.clone(),
            selected_monitor: self.selected_monitor().map(|monitor| monitor.id.clone()),
            auto_arrange: self.config.auto_arrange,
            reserve_two_slots: self.config.reserve_two_slots,
            default_application_mode: self.config.default_application_mode,
            aspect_ratio: self
                .config
                .table_aspect_ratio
                .unwrap_or(DEFAULT_ASPECT_RATIO),
            status_message: self.status_message.clone(),
            hotkeys: self.config.hotkeys.clone(),
        };
        if self.last_published.as_deref() == Some(&snapshot) {
            return;
        }

        while self.stale_snapshot_rx.try_recv().is_ok() {}
        let snapshot = Arc::new(snapshot);
        if self.snapshot_tx.try_send(Arc::clone(&snapshot)).is_ok() {
            self.last_published = Some(snapshot);
            (self.wake_ui)();
        }
    }
}
