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
    layout::{
        DEFAULT_ASPECT_RATIO, PokerColumnSpec, calculate_mixed_layout,
        normalized_ldplayer_aspect_ratio, right_side_free_rect,
    },
    model::{
        BackendError, CandidateDisposition, CandidateView, ManagedTable, MonitorInfo,
        PokerClientKind, PokerColumnAssignment, PokerSlotId, PokerSlotView, Rect, TableStatus,
        UiSnapshot, WindowBackend, WindowCandidate, WindowId, WindowMode, WindowSignature,
    },
};

const FALLBACK_RECONCILE_INTERVAL: Duration = Duration::from_secs(10);
const DISCOVERY_DEBOUNCE: Duration = Duration::from_millis(200);
const REFLOW_DEBOUNCE: Duration = Duration::from_millis(500);
const LOBBY_LOCATE_GAP: Duration = Duration::from_millis(80);
const COMMAND_QUEUE_CAPACITY: usize = 64;
const SNAPSHOT_QUEUE_CAPACITY: usize = 1;
const CONTROLLER_STACK_BYTES: usize = 512 * 1024;

pub type UiWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug)]
pub enum ControllerCommand {
    ForceArrange,
    NativeWindowEvent(&'static AtomicBool),
    SetAutoArrange(bool),
    SetPreserveTableSlots(bool),
    SetDefaultApplicationMode(ApplicationDefault),
    SetEnabled {
        id: WindowId,
        enabled: bool,
    },
    ToggleFocused,
    ToggleSlot(usize),
    MoveToSlot {
        source: WindowId,
        destination: PokerSlotId,
    },
    SelectMonitor(String),
    SetWindowMode {
        id: WindowId,
        mode: WindowMode,
    },
    SetClubGgLobbiesMode(WindowMode),
    LocateClubGgLobbies,
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
            status_message: "Looking for poker tables…".to_owned(),
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
            ControllerCommand::SetPreserveTableSlots(enabled) => {
                self.config.preserve_table_slots = enabled;
                self.sync_poker_columns();
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
            ControllerCommand::MoveToSlot {
                source,
                destination,
            } => {
                if self.move_to_slot(source, destination) {
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
            ControllerCommand::SetClubGgLobbiesMode(mode) => {
                self.set_clubgg_lobbies_mode(mode);
            }
            ControllerCommand::LocateClubGgLobbies => self.locate_clubgg_lobbies(),
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
        let Some((signature, poker_client)) = self
            .candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| (candidate.signature.clone(), candidate.poker_client))
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
        if poker_client == Some(PokerClientKind::LdPlayer) {
            self.config
                .set_application_disposition(signature, disposition);
        } else if poker_client.is_some() {
            self.config.set_disposition(signature, disposition);
        } else {
            self.config
                .set_application_disposition(signature, disposition);
        }
        if mode == WindowMode::Ignored {
            self.window_statuses.remove(&id);
            self.release_slot_for(id);
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

    fn set_clubgg_lobbies_mode(&mut self, mode: WindowMode) {
        let disposition = match mode {
            WindowMode::Parked => CandidateDisposition::Parked,
            WindowMode::TopRight => CandidateDisposition::TopRight,
            WindowMode::Ignored => CandidateDisposition::Ignored,
            WindowMode::Arranged | WindowMode::FreeSpace => return,
        };
        let signatures: HashSet<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.is_clubgg_lobby)
            .map(|candidate| candidate.signature.clone())
            .collect();
        if signatures.is_empty() {
            return;
        }
        for signature in signatures {
            self.config.set_disposition(signature, disposition);
        }
        self.save_config();
        self.reconcile();
        self.arrange();
    }

    fn locate_clubgg_lobbies(&mut self) {
        let lobbies: Vec<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.is_clubgg_lobby)
            .map(|candidate| candidate.id)
            .collect();
        let mut failed = 0_usize;
        for (index, id) in lobbies.iter().enumerate() {
            let mut lobby_failed = false;
            if let Err(error) = self.backend.locate(*id) {
                self.window_statuses
                    .insert(*id, TableStatus::MoveFailed(error.to_string()));
                lobby_failed = true;
            } else if !lobby_failed {
                self.window_statuses.insert(*id, TableStatus::Ready);
            }
            failed += usize::from(lobby_failed);
            if index + 1 < lobbies.len() {
                thread::sleep(LOBBY_LOCATE_GAP);
            }
        }
        self.status_message = if failed == 0 {
            format!(
                "Located {} ClubGG lobb{}.",
                lobbies.len(),
                if lobbies.len() == 1 { "y" } else { "ies" }
            )
        } else {
            format!(
                "Located {} of {} ClubGG lobbies.",
                lobbies.len().saturating_sub(failed),
                lobbies.len()
            )
        };
        self.publish();
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
        let old_candidate_ids: HashSet<_> = self
            .candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect();
        let old_positioned_ids = self.positioned_window_ids();
        let candidate_by_id: HashMap<_, _> =
            candidates.iter().map(|item| (item.id, item)).collect();

        self.tables
            .retain(|table| candidate_by_id.contains_key(&table.id));
        for table in &mut self.tables {
            if let Some(candidate) = candidate_by_id.get(&table.id) {
                table.label.clone_from(&candidate.label);
                table.signature.clone_from(&candidate.signature);
                table.current_rect = candidate.rect;
                if candidate.poker_client == Some(PokerClientKind::LdPlayer) {
                    table.preferred_aspect_ratio = candidate.preferred_aspect_ratio;
                }
                if table.enabled {
                    table.last_active_rect = candidate.rect;
                }
            }
        }

        let mut table_order_changed = false;
        for candidate in &candidates {
            let disposition = self.disposition_for_candidate(candidate);
            let should_manage = !candidate.is_clubgg_lobby
                && match disposition {
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
                    poker_client: candidate.poker_client.unwrap_or(PokerClientKind::ClubGg),
                    preferred_aspect_ratio: match candidate.poker_client {
                        Some(PokerClientKind::LdPlayer) => candidate.preferred_aspect_ratio,
                        _ => DEFAULT_ASPECT_RATIO,
                    },
                    enabled,
                    last_active_rect: candidate.rect,
                    current_rect: candidate.rect,
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
        let slots_changed = self.sync_poker_columns();
        if table_order_changed || slots_changed {
            self.save_config();
        }

        let new_ids: HashSet<_> = self.tables.iter().map(|table| table.id).collect();
        let new_candidate_ids: HashSet<_> =
            candidates.iter().map(|candidate| candidate.id).collect();
        let table_set_changed = old_ids != new_ids;
        let candidate_set_changed = old_candidate_ids != new_candidate_ids;
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
                .filter(|table| table.poker_client == PokerClientKind::ClubGg)
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
            || candidate_set_changed
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
                !candidate.is_clubgg_lobby
                    && (candidate.likely_table
                        || self.tables.iter().any(|table| table.id == candidate.id))
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
        let saved = self.config.disposition_for(&candidate.signature);
        if candidate.is_clubgg_lobby {
            return Some(match saved {
                Some(CandidateDisposition::Table) | None => CandidateDisposition::Parked,
                Some(disposition) => disposition,
            });
        }
        saved.or_else(|| {
            (candidate.poker_client.is_none())
                .then(|| self.config.default_application_mode.disposition())
        })
    }

    fn sync_poker_columns(&mut self) -> bool {
        let before = self.config.poker_columns.clone();
        let live: HashMap<_, _> = self
            .tables
            .iter()
            .map(|table| (table.signature.clone(), table.poker_client))
            .collect();

        for column in &mut self.config.poker_columns {
            match column {
                PokerColumnAssignment::ClubGg { top, bottom } => {
                    if top
                        .as_ref()
                        .is_some_and(|signature| !live.contains_key(signature))
                    {
                        *top = None;
                    }
                    if bottom
                        .as_ref()
                        .is_some_and(|signature| !live.contains_key(signature))
                    {
                        *bottom = None;
                    }
                }
                PokerColumnAssignment::LdPlayer { table } => {
                    if table
                        .as_ref()
                        .is_some_and(|signature| !live.contains_key(signature))
                    {
                        *column = PokerColumnAssignment::Empty;
                    }
                }
                PokerColumnAssignment::Empty => {}
            }
        }

        if !self.config.preserve_table_slots {
            self.config.poker_columns = compact_columns(self.tables.iter());
        } else {
            while self.config.poker_columns.len() < 2 {
                self.config
                    .poker_columns
                    .push(PokerColumnAssignment::empty_club());
            }
            let mut assigned: HashSet<_> = self
                .config
                .poker_columns
                .iter()
                .flat_map(column_signatures)
                .cloned()
                .collect();
            let mut pending: Vec<_> = self
                .tables
                .iter()
                .filter(|table| !assigned.contains(&table.signature))
                .map(|table| (table.signature.clone(), table.poker_client))
                .collect();
            pending.sort_by_key(|(_, client)| match client {
                PokerClientKind::ClubGg => 0_u8,
                PokerClientKind::LdPlayer => 1_u8,
            });
            for (signature, client) in pending {
                assign_new_signature(&mut self.config.poker_columns, signature.clone(), client);
                assigned.insert(signature);
            }
            normalize_preserved_columns(&mut self.config.poker_columns);
        }

        self.sort_tables_by_columns();
        self.config.table_order = self
            .tables
            .iter()
            .map(|table| table.signature.clone())
            .collect();
        before != self.config.poker_columns
    }

    fn sort_tables_by_columns(&mut self) {
        let order: HashMap<_, _> = self
            .config
            .poker_columns
            .iter()
            .flat_map(column_signatures)
            .cloned()
            .enumerate()
            .map(|(index, signature)| (signature, index))
            .collect();
        self.tables.sort_by_key(|table| {
            order
                .get(&table.signature)
                .map_or((1_u8, usize::MAX), |index| (0_u8, *index))
        });
    }

    fn release_slot_for(&mut self, id: WindowId) {
        let Some(signature) = self
            .tables
            .iter()
            .find(|table| table.id == id)
            .map(|table| table.signature.clone())
        else {
            return;
        };
        clear_signature(&mut self.config.poker_columns, &signature);
    }

    fn move_to_slot(&mut self, source: WindowId, destination: PokerSlotId) -> bool {
        let Some(source_table) = self.tables.iter().find(|table| table.id == source) else {
            return false;
        };
        let source_signature = source_table.signature.clone();
        let source_client = source_table.poker_client;
        let Some(source_slot) = find_signature_slot(&self.config.poker_columns, &source_signature)
        else {
            return false;
        };
        if source_slot == destination || destination.column >= self.config.poker_columns.len() {
            return false;
        }

        let destination_client = signature_at_slot(&self.config.poker_columns, destination)
            .and_then(|signature| {
                self.tables
                    .iter()
                    .find(|table| &table.signature == signature)
                    .map(|table| table.poker_client)
            });
        let whole_column = source_client == PokerClientKind::LdPlayer
            || destination_client == Some(PokerClientKind::LdPlayer)
            || destination.row.is_none();

        if whole_column {
            self.config
                .poker_columns
                .swap(source_slot.column, destination.column);
        } else {
            let destination_signature =
                signature_at_slot(&self.config.poker_columns, destination).cloned();
            set_slot_signature(
                &mut self.config.poker_columns,
                source_slot,
                destination_signature,
            );
            set_slot_signature(
                &mut self.config.poker_columns,
                destination,
                Some(source_signature),
            );
        }
        self.sort_tables_by_columns();
        true
    }

    fn runtime_columns(&self) -> Vec<PokerColumnAssignment> {
        if self.effective_preserve_table_slots() {
            self.config.poker_columns.clone()
        } else if self.config.preserve_table_slots {
            active_assigned_columns(&self.config.poker_columns, &self.tables)
        } else {
            compact_columns(self.tables.iter().filter(|table| table.enabled))
        }
    }

    fn effective_preserve_table_slots(&self) -> bool {
        self.config.preserve_table_slots && self.occupied_active_column_count() < 2
    }

    fn occupied_active_column_count(&self) -> usize {
        let enabled: HashSet<_> = self
            .tables
            .iter()
            .filter(|table| table.enabled)
            .map(|table| &table.signature)
            .collect();
        self.config
            .poker_columns
            .iter()
            .filter(|column| column_signatures(column).any(|signature| enabled.contains(signature)))
            .count()
    }

    fn mixed_layout_for(
        &self,
        monitor: &MonitorInfo,
        columns: &[PokerColumnAssignment],
    ) -> crate::layout::MixedPokerLayout {
        let specs: Vec<_> = columns
            .iter()
            .map(|column| match column {
                PokerColumnAssignment::LdPlayer {
                    table: Some(signature),
                } => {
                    let ratio = normalized_ldplayer_aspect_ratio(
                        self.tables
                            .iter()
                            .find(|item| &item.signature == signature)
                            .map_or(0.0, |item| item.preferred_aspect_ratio),
                    );
                    PokerColumnSpec::LdPlayer {
                        aspect_ratio: ratio,
                    }
                }
                PokerColumnAssignment::LdPlayer { table: None }
                | PokerColumnAssignment::ClubGg { .. }
                | PokerColumnAssignment::Empty => PokerColumnSpec::ClubGg,
            })
            .collect();
        calculate_mixed_layout(monitor.work_area, &specs)
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
        let columns = self.runtime_columns();
        let layout = self.mixed_layout_for(&monitor, &columns);
        let enabled_count = self.tables.iter().filter(|table| table.enabled).count();
        info!(
            "arrangement started; active_tables={}; columns={}; height={}",
            enabled_count,
            layout.columns.len(),
            layout.height
        );

        let mut requests = Vec::new();
        for (column_index, assignment) in columns.iter().enumerate() {
            let Some(column_layout) = layout.columns.get(column_index) else {
                continue;
            };
            match assignment {
                PokerColumnAssignment::ClubGg { top, bottom } => {
                    if let (Some(signature), Some(rect)) = (top, column_layout.top) {
                        requests.push((signature.clone(), rect));
                    }
                    if let (Some(signature), Some(rect)) = (bottom, column_layout.bottom) {
                        requests.push((signature.clone(), rect));
                    }
                }
                PokerColumnAssignment::LdPlayer {
                    table: Some(signature),
                } => requests.push((signature.clone(), column_layout.bounds)),
                PokerColumnAssignment::LdPlayer { table: None } | PokerColumnAssignment::Empty => {}
            }
        }

        let mut moved_count = 0_usize;
        let mut failed_count = 0_usize;
        let mut access_denied_count = 0_usize;
        for (slot, (signature, requested)) in requests.into_iter().enumerate() {
            let Some(index) = self
                .tables
                .iter()
                .position(|table| table.signature == signature && table.enabled)
            else {
                continue;
            };
            let ratio = self.tables[index].preferred_aspect_ratio;
            if let Ok(minimum) = self.backend.minimum_size(self.tables[index].id, ratio)
                && (requested.width < minimum.width || requested.height < minimum.height)
            {
                self.tables[index].status = TableStatus::MoveFailed(
                    "The mixed layout is smaller than this window's supported minimum".to_owned(),
                );
                failed_count += 1;
                continue;
            }
            match self.backend.move_resize(self.tables[index].id, requested) {
                Ok(actual) => {
                    self.tables[index].last_active_rect = actual;
                    self.tables[index].current_rect = actual;
                    self.tables[index].status = TableStatus::Ready;
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

        self.park_all();

        let (positioned_requested, positioned_moved, positioned_failed, positioned_denied) =
            self.position_other_windows(&monitor);
        failed_count += positioned_failed;
        access_denied_count += positioned_denied;
        let requested_count = enabled_count + positioned_requested;
        let total_moved = moved_count + positioned_moved;

        self.status_message = if requested_count == 0 {
            "No windows selected for positioning.".to_owned()
        } else if failed_count == 0 && enabled_count == 0 {
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
            enabled_count, positioned_requested, total_moved, failed_count
        );
        self.publish();
    }

    fn position_other_windows(&mut self, monitor: &MonitorInfo) -> (usize, usize, usize, usize) {
        let columns = self.runtime_columns();
        let layout = self.mixed_layout_for(monitor, &columns);
        let occupied: Vec<_> = layout.columns.iter().map(|column| column.bounds).collect();
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
        let parked_tables: Vec<_> = self
            .tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| (!table.enabled).then_some(index))
            .collect();
        let parked_lobbies: Vec<_> = self
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                (candidate.is_clubgg_lobby
                    && self.disposition_for_candidate(candidate)
                        == Some(CandidateDisposition::Parked))
                .then_some(index)
            })
            .collect();
        let parked: Vec<_> = parked_tables
            .into_iter()
            .map(|index| (Some(index), None))
            .chain(parked_lobbies.into_iter().map(|index| (None, Some(index))))
            .collect();
        let mut cursor_right = monitor.work_area.right();
        let mut cursor_bottom = monitor.work_area.bottom();
        let mut row_height = 0_i32;

        for (table_index, lobby_index) in parked {
            let (id, aspect_ratio) = if let Some(index) = table_index {
                (
                    self.tables[index].id,
                    self.tables[index].preferred_aspect_ratio,
                )
            } else if let Some(index) = lobby_index {
                (
                    self.candidates[index].id,
                    self.candidates[index].preferred_aspect_ratio,
                )
            } else {
                continue;
            };
            let size = self
                .backend
                .minimum_size(id, aspect_ratio)
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
            match self.backend.move_resize(id, rect) {
                Ok(actual) => {
                    if let Some(index) = table_index {
                        self.tables[index].status = TableStatus::Parked;
                        self.tables[index].current_rect = actual;
                    } else if let Some(index) = lobby_index {
                        self.candidates[index].rect = actual;
                        self.window_statuses.insert(id, TableStatus::Parked);
                    }
                    cursor_right = actual.left;
                    row_height = row_height.max(actual.height);
                }
                Err(BackendError::AccessDenied) => {
                    if let Some(index) = table_index {
                        self.tables[index].status = TableStatus::AccessDenied;
                    } else {
                        self.window_statuses.insert(id, TableStatus::AccessDenied);
                    }
                    cursor_right = rect.left;
                    row_height = row_height.max(height);
                }
                Err(error) => {
                    let status = TableStatus::MoveFailed(error.to_string());
                    if let Some(index) = table_index {
                        self.tables[index].status = status;
                    } else {
                        self.window_statuses.insert(id, status);
                    }
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
        self.config.table_order = self
            .tables
            .iter()
            .map(|table| table.signature.clone())
            .collect();
        self.save_config();
    }

    fn poker_slot_views(&self) -> (Vec<PokerSlotView>, Option<Rect>) {
        let Some(monitor) = self.selected_monitor() else {
            return (Vec::new(), None);
        };
        let columns = self.runtime_columns();
        let layout = self.mixed_layout_for(monitor, &columns);
        let mut views = Vec::new();
        for (column_index, assignment) in columns.iter().enumerate() {
            let Some(column_layout) = layout.columns.get(column_index) else {
                continue;
            };
            match assignment {
                PokerColumnAssignment::ClubGg { top, bottom } => {
                    for (row, signature, rect) in [
                        (0, top.as_ref(), column_layout.top),
                        (1, bottom.as_ref(), column_layout.bottom),
                    ] {
                        let occupant = signature.and_then(|signature| {
                            self.tables
                                .iter()
                                .find(|table| &table.signature == signature)
                        });
                        if let Some(rect) = rect {
                            views.push(PokerSlotView {
                                id: PokerSlotId::club(column_index, row),
                                rect,
                                occupant: occupant.map(|table| table.id),
                                parked: occupant.is_some_and(|table| !table.enabled),
                            });
                        }
                    }
                }
                PokerColumnAssignment::LdPlayer { table } => {
                    let occupant = table.as_ref().and_then(|signature| {
                        self.tables.iter().find(|item| &item.signature == signature)
                    });
                    views.push(PokerSlotView {
                        id: PokerSlotId::full_height(column_index),
                        rect: column_layout.bounds,
                        occupant: occupant.map(|table| table.id),
                        parked: occupant.is_some_and(|table| !table.enabled),
                    });
                }
                PokerColumnAssignment::Empty => {
                    if let (Some(top), Some(bottom)) = (column_layout.top, column_layout.bottom) {
                        views.push(PokerSlotView {
                            id: PokerSlotId::club(column_index, 0),
                            rect: top,
                            occupant: None,
                            parked: false,
                        });
                        views.push(PokerSlotView {
                            id: PokerSlotId::club(column_index, 1),
                            rect: bottom,
                            occupant: None,
                            parked: false,
                        });
                    }
                }
            }
        }
        (views, Some(monitor.work_area))
    }

    fn save_config(&mut self) {
        if let Err(error) = self.store.save(&self.config) {
            warn!("could not save configuration: {error}");
            self.status_message = format!("Settings could not be saved: {error}");
        }
    }

    fn publish(&mut self) {
        let (poker_slots, poker_work_area) = self.poker_slot_views();
        let preserve_table_slots = self.effective_preserve_table_slots();
        let candidates = self
            .candidates
            .iter()
            .map(|candidate| CandidateView {
                id: candidate.id,
                label: candidate.label.clone(),
                process_name: candidate.process_name.clone(),
                class_name: candidate.class_name.clone(),
                poker_client: candidate.poker_client,
                is_clubgg_lobby: candidate.is_clubgg_lobby,
                current_rect: self
                    .tables
                    .iter()
                    .find(|table| table.id == candidate.id)
                    .map_or(candidate.rect, |table| table.current_rect),
                likely_table: candidate.likely_table,
                mode: self
                    .tables
                    .iter()
                    .find(|table| table.id == candidate.id)
                    .map_or_else(
                        || match self.disposition_for_candidate(candidate) {
                            Some(CandidateDisposition::Parked) => WindowMode::Parked,
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
            poker_slots,
            poker_work_area,
            monitors: self.monitors.clone(),
            selected_monitor: self.selected_monitor().map(|monitor| monitor.id.clone()),
            auto_arrange: self.config.auto_arrange,
            preserve_table_slots,
            preserve_table_slots_requested: self.config.preserve_table_slots,
            preserve_table_slots_auto_suppressed: self.config.preserve_table_slots
                && !preserve_table_slots,
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

fn compact_columns<'a>(
    tables: impl Iterator<Item = &'a ManagedTable>,
) -> Vec<PokerColumnAssignment> {
    let mut clubgg = Vec::new();
    let mut ldplayer = Vec::new();
    for table in tables {
        match table.poker_client {
            PokerClientKind::ClubGg => clubgg.push(table.signature.clone()),
            PokerClientKind::LdPlayer => ldplayer.push(table.signature.clone()),
        }
    }

    let mut columns = Vec::new();
    for pair in clubgg.chunks(2) {
        columns.push(PokerColumnAssignment::ClubGg {
            top: pair.first().cloned(),
            bottom: pair.get(1).cloned(),
        });
    }
    columns.extend(
        ldplayer
            .into_iter()
            .map(|table| PokerColumnAssignment::LdPlayer { table: Some(table) }),
    );
    columns
}

fn active_assigned_columns(
    columns: &[PokerColumnAssignment],
    tables: &[ManagedTable],
) -> Vec<PokerColumnAssignment> {
    let enabled: HashSet<_> = tables
        .iter()
        .filter(|table| table.enabled)
        .map(|table| &table.signature)
        .collect();
    columns
        .iter()
        .filter_map(|column| match column {
            PokerColumnAssignment::ClubGg { top, bottom } => {
                let top = top
                    .as_ref()
                    .filter(|signature| enabled.contains(signature))
                    .cloned();
                let bottom = bottom
                    .as_ref()
                    .filter(|signature| enabled.contains(signature))
                    .cloned();
                (top.is_some() || bottom.is_some())
                    .then_some(PokerColumnAssignment::ClubGg { top, bottom })
            }
            PokerColumnAssignment::LdPlayer { table } => table
                .as_ref()
                .filter(|signature| enabled.contains(signature))
                .cloned()
                .map(|table| PokerColumnAssignment::LdPlayer { table: Some(table) }),
            PokerColumnAssignment::Empty => None,
        })
        .collect()
}

fn column_signatures(column: &PokerColumnAssignment) -> impl Iterator<Item = &WindowSignature> {
    let mut signatures = [None, None];
    match column {
        PokerColumnAssignment::ClubGg { top, bottom } => {
            signatures[0] = top.as_ref();
            signatures[1] = bottom.as_ref();
        }
        PokerColumnAssignment::LdPlayer { table } => signatures[0] = table.as_ref(),
        PokerColumnAssignment::Empty => {}
    }
    signatures.into_iter().flatten()
}

fn assign_new_signature(
    columns: &mut Vec<PokerColumnAssignment>,
    signature: WindowSignature,
    client: PokerClientKind,
) {
    match client {
        PokerClientKind::ClubGg => {
            let club_section_end = columns
                .iter()
                .position(|column| matches!(column, PokerColumnAssignment::LdPlayer { .. }))
                .unwrap_or(columns.len());
            for column in &mut columns[..club_section_end] {
                match column {
                    PokerColumnAssignment::ClubGg { top, bottom } => {
                        if top.is_none() {
                            *top = Some(signature);
                            return;
                        }
                        if bottom.is_none() {
                            *bottom = Some(signature);
                            return;
                        }
                    }
                    PokerColumnAssignment::Empty => {
                        *column = PokerColumnAssignment::ClubGg {
                            top: Some(signature),
                            bottom: None,
                        };
                        return;
                    }
                    PokerColumnAssignment::LdPlayer { .. } => {}
                }
            }
            columns.insert(
                club_section_end,
                PokerColumnAssignment::ClubGg {
                    top: Some(signature),
                    bottom: None,
                },
            );
        }
        PokerClientKind::LdPlayer => {
            let after_last_club = columns
                .iter()
                .rposition(|column| {
                    matches!(column, PokerColumnAssignment::ClubGg { .. })
                        && column_is_owned(column)
                })
                .map_or(0, |index| index + 1);
            if let Some(column) = columns[after_last_club..]
                .iter_mut()
                .find(|column| !column_is_owned(column))
            {
                *column = PokerColumnAssignment::LdPlayer {
                    table: Some(signature),
                };
                return;
            }
            columns.push(PokerColumnAssignment::LdPlayer {
                table: Some(signature),
            });
        }
    }
}

fn normalize_preserved_columns(columns: &mut Vec<PokerColumnAssignment>) {
    let owned = columns
        .iter()
        .filter(|column| column_is_owned(column))
        .count();
    let mut anonymous_left = 2_usize.saturating_sub(owned);
    columns.retain(|column| {
        if column_is_owned(column) {
            true
        } else if anonymous_left > 0 {
            anonymous_left -= 1;
            true
        } else {
            false
        }
    });
    while columns.len() < 2 {
        columns.push(PokerColumnAssignment::empty_club());
    }
}

fn column_is_owned(column: &PokerColumnAssignment) -> bool {
    column_signatures(column).next().is_some()
}

fn find_signature_slot(
    columns: &[PokerColumnAssignment],
    signature: &WindowSignature,
) -> Option<PokerSlotId> {
    columns
        .iter()
        .enumerate()
        .find_map(|(column, assignment)| match assignment {
            PokerColumnAssignment::ClubGg { top, bottom } if top.as_ref() == Some(signature) => {
                Some(PokerSlotId::club(column, 0))
            }
            PokerColumnAssignment::ClubGg { top: _, bottom }
                if bottom.as_ref() == Some(signature) =>
            {
                Some(PokerSlotId::club(column, 1))
            }
            PokerColumnAssignment::LdPlayer { table } if table.as_ref() == Some(signature) => {
                Some(PokerSlotId::full_height(column))
            }
            _ => None,
        })
}

fn signature_at_slot(
    columns: &[PokerColumnAssignment],
    slot: PokerSlotId,
) -> Option<&WindowSignature> {
    match columns.get(slot.column)? {
        PokerColumnAssignment::ClubGg { top, bottom } => match slot.row {
            Some(0) => top.as_ref(),
            Some(1) => bottom.as_ref(),
            _ => None,
        },
        PokerColumnAssignment::LdPlayer { table } if slot.row.is_none() => table.as_ref(),
        PokerColumnAssignment::LdPlayer { .. } | PokerColumnAssignment::Empty => None,
    }
}

fn set_slot_signature(
    columns: &mut [PokerColumnAssignment],
    slot: PokerSlotId,
    signature: Option<WindowSignature>,
) {
    let Some(column) = columns.get_mut(slot.column) else {
        return;
    };
    match (column, slot.row) {
        (PokerColumnAssignment::ClubGg { top, .. }, Some(0)) => *top = signature,
        (PokerColumnAssignment::ClubGg { bottom, .. }, Some(1)) => *bottom = signature,
        (PokerColumnAssignment::LdPlayer { table }, None) => *table = signature,
        (empty @ PokerColumnAssignment::Empty, Some(row)) => {
            *empty = if row == 0 {
                PokerColumnAssignment::ClubGg {
                    top: signature,
                    bottom: None,
                }
            } else {
                PokerColumnAssignment::ClubGg {
                    top: None,
                    bottom: signature,
                }
            };
        }
        _ => {}
    }
}

fn clear_signature(columns: &mut [PokerColumnAssignment], signature: &WindowSignature) {
    if let Some(slot) = find_signature_slot(columns, signature) {
        set_slot_signature(columns, slot, None);
        if matches!(
            columns[slot.column],
            PokerColumnAssignment::LdPlayer { table: None }
        ) {
            columns[slot.column] = PokerColumnAssignment::Empty;
        }
    }
}
