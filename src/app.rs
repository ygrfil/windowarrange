use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use eframe::egui;
use log::{error, info, warn};

use crate::{
    config::{AppConfig, ApplicationDefault, ConfigStore, HotkeySettings},
    controller::{ControllerCommand, ControllerHandle, spawn_controller_with_waker},
    hotkeys::{HotkeyAction, HotkeyService},
    identity::{APP_ID, PANEL_TITLE},
    logging,
    model::{
        CandidateView, PokerClientKind, PokerSlotId, PokerSlotView, Rect, TableStatus, UiSnapshot,
        WindowId, WindowMode,
    },
    tray::{TrayAction, TrayService, egui_icon},
    win32::{
        Win32Backend, acquire_single_instance, activate_existing_panel, apply_process_mitigations,
        spawn_window_event_watcher,
    },
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(22, 163, 74);
const PARKED: egui::Color32 = egui::Color32::from_rgb(217, 119, 6);
const TOP_RIGHT: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);
const FREE_SPACE: egui::Color32 = egui::Color32::from_rgb(124, 58, 237);
const BACKGROUND_EVENT_QUEUE_CAPACITY: usize = 16;
const MIRROR_MIN_HEIGHT: f32 = 118.0;
const MIRROR_MAX_PANE_WIDTH: f32 = 370.0;
const APPLICATION_TILE_HEIGHT: f32 = 44.0;
const PLACEHOLDER_BADGE: egui::Color32 = egui::Color32::from_rgb(70, 94, 78);
const PLACEHOLDER_LABEL: &str = "Placeholders";
const CARD_ROW_GAP: f32 = 4.0;
const MINIMUM_BOARD_HEIGHT: f32 = 62.0;
const PANEL_CHROME_HEIGHT: f32 = 84.0;
const SETTINGS_WIDTH: f32 = 440.0;
const SETTINGS_MIN_HEIGHT: f32 = 240.0;
const SETTINGS_MAX_HEIGHT: f32 = 700.0;

pub fn run() {
    let mitigation_result = apply_process_mitigations();
    let store = ConfigStore::for_current_user();
    let log_path = store.log_path();
    let logging_result = logging::initialize(&log_path);
    std::panic::set_hook(Box::new(|panic| {
        error!("application panic: {panic}");
    }));
    match logging_result {
        Ok(path) => info!(
            "Table Arranger Control started; version={}; log_file={}",
            env!("CARGO_PKG_VERSION"),
            path.display()
        ),
        Err(error) => {
            eprintln!("Could not initialize file logging: {error}");
        }
    }
    match mitigation_result {
        Ok(()) => info!("legacy extension-point DLL loading is disabled"),
        Err(error) => warn!("could not enable process extension-point mitigation: {error}"),
    }

    let _instance = match acquire_single_instance() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            activate_existing_panel();
            return;
        }
        Err(error) => {
            error!("single-instance setup failed: {error}");
            return;
        }
    };

    let config = match store.load() {
        Ok(config) => config,
        Err(error) => {
            warn!("using default configuration: {error}");
            AppConfig::default()
        }
    };
    let hotkey_settings = config.hotkeys.clone();
    let backend = Win32Backend::new();

    let app_icon = Arc::new(egui_icon());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(PANEL_TITLE)
            .with_inner_size([740.0, 260.0])
            .with_min_inner_size([680.0, 180.0])
            .with_max_inner_size([820.0, 420.0])
            .with_maximize_button(false)
            .with_always_on_top()
            .with_icon(Arc::clone(&app_icon)),
        centered: false,
        persist_window: true,
        persistence_path: Some(store.ui_state_path()),
        ..Default::default()
    };

    let result = eframe::run_native(
        PANEL_TITLE,
        options,
        Box::new(move |creation| {
            let repaint_context = creation.egui_ctx.clone();
            let controller = spawn_controller_with_waker(
                backend,
                config,
                store,
                Arc::new(move || repaint_context.request_repaint()),
            );
            spawn_window_event_watcher(controller.commands.clone());
            Ok(Box::new(TableArrangerApp::new(
                creation,
                controller,
                hotkey_settings,
                app_icon,
            )))
        }),
    );
    if let Err(error) = result {
        error!("application exited with an error: {error}");
    }
}

struct TableArrangerApp {
    commands: Sender<ControllerCommand>,
    snapshots: Receiver<Arc<UiSnapshot>>,
    snapshot: Arc<UiSnapshot>,
    hotkeys: Option<HotkeyService>,
    hotkey_events: Receiver<u32>,
    _tray: Option<TrayService>,
    tray_events: Receiver<TrayAction>,
    settings: Arc<Mutex<SettingsViewportState>>,
    settings_open: Arc<AtomicBool>,
    settings_focus_requested: bool,
    app_icon: Arc<egui::IconData>,
    selected_table: Option<crate::model::WindowId>,
    exiting: bool,
}

struct SettingsViewportState {
    snapshot: Arc<UiSnapshot>,
    shortcut_draft: HotkeySettings,
    shortcut_errors: Vec<String>,
    pending_hotkeys: Option<HotkeySettings>,
    desired_height: f32,
}

impl TableArrangerApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        controller: ControllerHandle,
        hotkey_settings: HotkeySettings,
        app_icon: Arc<egui::IconData>,
    ) -> Self {
        configure_style(&creation.egui_ctx);
        let (hotkey_tx, hotkey_rx) = bounded(BACKGROUND_EVENT_QUEUE_CAPACITY);
        let (tray_tx, tray_rx) = bounded(BACKGROUND_EVENT_QUEUE_CAPACITY);

        let (hotkeys, shortcut_errors) =
            match HotkeyService::new(&hotkey_settings, creation.egui_ctx.clone(), hotkey_tx) {
                Ok((service, errors)) => (Some(service), errors),
                Err(error) => (None, vec![error]),
            };
        let tray = match TrayService::new(creation.egui_ctx.clone(), tray_tx) {
            Ok(service) => Some(service),
            Err(error) => {
                warn!("tray icon unavailable: {error}");
                None
            }
        };

        let snapshot = Arc::new(UiSnapshot::default());
        let settings = SettingsViewportState {
            snapshot: Arc::clone(&snapshot),
            shortcut_draft: hotkey_settings,
            shortcut_errors,
            pending_hotkeys: None,
            desired_height: 300.0,
        };

        Self {
            commands: controller.commands,
            snapshots: controller.snapshots,
            snapshot,
            hotkeys,
            hotkey_events: hotkey_rx,
            _tray: tray,
            tray_events: tray_rx,
            settings: Arc::new(Mutex::new(settings)),
            settings_open: Arc::new(AtomicBool::new(false)),
            settings_focus_requested: false,
            app_icon,
            selected_table: None,
            exiting: false,
        }
    }

    fn send(&self, command: ControllerCommand) {
        let _ = self.commands.send(command);
    }

    fn show_panel(context: &egui::Context) {
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide_panel(context: &egui::Context) {
        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn handle_background_events(&mut self, context: &egui::Context) {
        for snapshot in self.snapshots.try_iter() {
            self.snapshot = Arc::clone(&snapshot);
            lock_settings(&self.settings).snapshot = snapshot;
            if self.settings_open.load(Ordering::Acquire) {
                context.request_repaint_of(settings_viewport_id());
            }
        }
        for id in self.hotkey_events.try_iter() {
            let Some(action) = self
                .hotkeys
                .as_ref()
                .and_then(|service| service.action_for(id))
            else {
                continue;
            };
            match action {
                HotkeyAction::ArrangeNow => self.send(ControllerCommand::ForceArrange),
                HotkeyAction::ToggleFocused => self.send(ControllerCommand::ToggleFocused),
                HotkeyAction::ShowPanel => Self::show_panel(context),
                HotkeyAction::ToggleSlot(slot) => {
                    self.send(ControllerCommand::ToggleSlot(slot));
                }
            }
        }
        for action in self.tray_events.try_iter() {
            match action {
                TrayAction::ShowPanel => Self::show_panel(context),
                TrayAction::ArrangeNow => self.send(ControllerCommand::ForceArrange),
                TrayAction::Exit => {
                    self.exiting = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        let pending_hotkeys = lock_settings(&self.settings).pending_hotkeys.take();
        if let Some(settings) = pending_hotkeys {
            let errors = self.hotkeys.as_mut().map_or_else(
                || vec!["Global hotkey service is unavailable.".to_owned()],
                |service| service.apply(&settings),
            );
            self.send(ControllerCommand::SetHotkeys(settings));
            lock_settings(&self.settings).shortcut_errors = errors;
            context.request_repaint_of(settings_viewport_id());
        }

        if context.input(|input| input.viewport().close_requested()) && !self.exiting {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            Self::hide_panel(context);
        }
        if context.input(|input| input.viewport().minimized == Some(true)) && !self.exiting {
            context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            Self::hide_panel(context);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            let mut automatic = self.snapshot.auto_arrange;
            if ui
                .add_sized(
                    [64.0, 24.0],
                    egui::Button::new(if automatic { "Auto ON" } else { "Auto OFF" })
                        .selected(automatic),
                )
                .on_hover_text("Automatically reapply the workspace after window changes")
                .clicked()
            {
                automatic = !automatic;
                self.send(ControllerCommand::SetAutoArrange(automatic));
            }
            if ui
                .add_sized(
                    [64.0, 24.0],
                    egui::Button::new(egui::RichText::new("Arrange").color(egui::Color32::WHITE))
                        .fill(ACCENT)
                        .corner_radius(6),
                )
                .on_hover_text("Discover windows and immediately reapply all selected placement")
                .clicked()
            {
                self.send(ControllerCommand::ForceArrange);
            }
            if ui
                .add_sized([64.0, 24.0], egui::Button::new("Settings"))
                .on_hover_text("Workspace, display, defaults, and hotkeys")
                .clicked()
            {
                self.settings_open.store(true, Ordering::Release);
                self.settings_focus_requested = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized([26.0, 24.0], egui::Button::new("-"))
                    .on_hover_text("Hide to tray")
                    .clicked()
                {
                    Self::hide_panel(ui.ctx());
                }
            });
        })
        .response
    }

    fn workspace(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        if self.snapshot.candidates.is_empty() && self.snapshot.poker_slots.is_empty() {
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(10)
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No application windows found").strong());
                        ui.label(
                            egui::RichText::new("Open an application, then press Arrange.")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
            return;
        }

        let available = ui.available_size();
        let poker_width = (available.x * 0.53).clamp(300.0, 370.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(poker_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(poker_width);
                    ui.set_height(available.y);
                    self.poker_board(ui);
                },
            );
            ui.separator();
            let application_width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(application_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(application_width);
                    ui.set_height(available.y);
                    self.application_board(ui);
                },
            );
        });
    }

    fn poker_board(&mut self, ui: &mut egui::Ui) {
        let windows: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.poker_client.is_some())
            .cloned()
            .collect();

        let Some(work_area) = self.snapshot.poker_work_area else {
            empty_section(ui, "No poker display", "Choose a display in Settings.");
            return;
        };
        let aspect = if work_area.width > 0 && work_area.height > 0 {
            work_area.width as f32 / work_area.height as f32
        } else {
            16.0 / 9.0
        };
        let board_width = ui.available_width();
        let board_height = (board_width / aspect).clamp(
            MIRROR_MIN_HEIGHT,
            ui.available_height().max(MIRROR_MIN_HEIGHT),
        );
        let (board_rect, _) =
            ui.allocate_exact_size(egui::vec2(board_width, board_height), egui::Sense::hover());
        ui.painter()
            .rect_filled(board_rect, 6.0, ui.visuals().faint_bg_color);
        ui.painter().rect_stroke(
            board_rect,
            6.0,
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );

        let slots = self.snapshot.poker_slots.clone();
        for slot in &slots {
            let rect = project_rect(slot.rect, work_area, board_rect);
            self.mirrored_slot(ui, rect, slot, &windows);
        }

        for window in windows
            .iter()
            .filter(|window| window.mode == WindowMode::Parked)
        {
            let rect = project_rect(window.current_rect, work_area, board_rect);
            let response = ui
                .interact(
                    rect.expand(2.0),
                    ui.make_persistent_id(("parked-window", window.id.0)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!(
                    "{}\nLeft click: Locate\nRight click: Unpark",
                    window.label
                ));
            ui.painter()
                .rect_filled(rect, 2.0, PARKED.gamma_multiply(0.22));
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.5, PARKED),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{} {}",
                    window.slot.unwrap_or_default(),
                    short_label(&window.label, 7)
                ),
                egui::FontId::proportional(7.0),
                ui.visuals().text_color(),
            );
            if response.clicked_by(egui::PointerButton::Primary) {
                self.send(ControllerCommand::Locate(window.id));
            } else if response.clicked_by(egui::PointerButton::Secondary) {
                self.set_window_mode(window, WindowMode::Arranged);
            }
        }

        let ignored: Vec<_> = windows
            .iter()
            .filter(|window| window.mode == WindowMode::Ignored)
            .cloned()
            .collect();
        if !ignored.is_empty() {
            ui.add_space(3.0);
            let chip_width = ((ui.available_width() - 4.0) / 2.0).max(100.0);
            for (row_index, row) in ignored.chunks(2).enumerate() {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for window in row {
                        let selected = self.selected_table == Some(window.id);
                        ui.allocate_ui_with_layout(
                            egui::vec2(chip_width, 16.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                egui::Frame::new()
                                    .fill(ui.visuals().faint_bg_color)
                                    .corner_radius(4)
                                    .inner_margin(egui::Margin::same(1))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .selectable_label(
                                                    selected,
                                                    format!(
                                                        "{} {}",
                                                        window.slot.map_or_else(
                                                            || "—".to_owned(),
                                                            |slot| slot.to_string()
                                                        ),
                                                        short_label(&window.label, 8)
                                                    ),
                                                )
                                                .on_hover_text(&window.label)
                                                .clicked()
                                            {
                                                self.selected_table =
                                                    if selected { None } else { Some(window.id) };
                                            }
                                            for (icon, mode, color, tooltip) in [
                                                (
                                                    ActionIcon::Locate,
                                                    None,
                                                    ACCENT,
                                                    "Locate this window",
                                                ),
                                                (
                                                    ActionIcon::Arrange,
                                                    Some(WindowMode::Arranged),
                                                    ACCENT,
                                                    "Arrange this table",
                                                ),
                                                (
                                                    ActionIcon::Park,
                                                    Some(WindowMode::Parked),
                                                    PARKED,
                                                    "Park this table",
                                                ),
                                                (
                                                    ActionIcon::Ignore,
                                                    Some(WindowMode::Ignored),
                                                    egui::Color32::GRAY,
                                                    "Ignore this table",
                                                ),
                                            ] {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(12.0, 12.0),
                                                    egui::Sense::hover(),
                                                );
                                                if icon_button(
                                                    ui,
                                                    rect,
                                                    icon,
                                                    mode == Some(window.mode),
                                                    color,
                                                    tooltip,
                                                )
                                                .clicked()
                                                {
                                                    if let Some(mode) = mode {
                                                        self.set_window_mode(window, mode);
                                                    } else {
                                                        self.send(ControllerCommand::Locate(
                                                            window.id,
                                                        ));
                                                    }
                                                }
                                            }
                                        });
                                    });
                            },
                        );
                    }
                });
                if row_index + 1 < ignored.len().div_ceil(2) {
                    ui.add_space(2.0);
                }
            }
        }

        if self
            .selected_table
            .is_some_and(|selected| !windows.iter().any(|window| window.id == selected))
        {
            self.selected_table = None;
        }
    }

    fn mirrored_slot(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        slot: &PokerSlotView,
        windows: &[CandidateView],
    ) {
        let occupant = slot
            .occupant
            .and_then(|id| windows.iter().find(|window| window.id == id));
        let response = ui.interact(
            rect,
            ui.make_persistent_id(("poker-slot", slot.id.column, slot.id.row)),
            egui::Sense::click(),
        );
        let Some(window) = occupant else {
            dashed_rect(
                ui.painter(),
                rect.shrink(1.0),
                ui.visuals().weak_text_color(),
            );
            let badge_rect =
                egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 3.0), egui::vec2(16.0, 16.0));
            ui.painter().rect_filled(badge_rect, 3.0, PLACEHOLDER_BADGE);
            ui.painter().text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                placeholder_number(slot.id),
                egui::FontId::proportional(8.0),
                egui::Color32::WHITE,
            );
            ui.painter().text(
                egui::pos2(badge_rect.right() + 3.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                PLACEHOLDER_LABEL,
                egui::FontId::proportional(8.0),
                ui.visuals().weak_text_color(),
            );
            if response
                .on_hover_text("Placeholders — select a table, then click here to move it")
                .clicked()
                && let Some(command) = slot_click_command(&mut self.selected_table, None, slot.id)
            {
                self.send(command);
            }
            return;
        };

        let is_selected = self.selected_table == Some(window.id);
        let fill = if is_selected {
            ACCENT.gamma_multiply(0.22)
        } else if slot.parked {
            PARKED.gamma_multiply(0.16)
        } else if ui.visuals().dark_mode {
            egui::Color32::from_gray(27)
        } else {
            egui::Color32::from_gray(248)
        };
        let stroke = if is_selected {
            egui::Stroke::new(2.0, ACCENT)
        } else if slot.parked {
            egui::Stroke::new(1.5, PARKED)
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        ui.painter()
            .rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);

        let badge_rect =
            egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 3.0), egui::vec2(16.0, 16.0));
        ui.painter()
            .rect_filled(badge_rect, 3.0, if slot.parked { PARKED } else { ACCENT });
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            window.slot.unwrap_or_default(),
            egui::FontId::proportional(8.0),
            egui::Color32::WHITE,
        );
        let client = match window.poker_client {
            Some(PokerClientKind::LdPlayer) => "LD",
            _ => "CG",
        };
        ui.painter().text(
            egui::pos2(badge_rect.right() + 3.0, badge_rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{client} {}", short_label(&window.label, 10)),
            egui::FontId::proportional(8.5),
            ui.visuals().text_color(),
        );

        let icon_size = ((rect.width() - 7.0) / 4.0).clamp(9.0, 16.0);
        let gap = 1.0;
        let total = icon_size * 4.0 + gap * 3.0;
        let mut action_clicked = false;
        if rect.width() >= total + 4.0 && rect.height() >= icon_size * 2.0 + 5.0 {
            let start = egui::pos2(
                rect.center().x - total / 2.0,
                rect.bottom() - icon_size - 2.0,
            );
            for (index, (icon, mode, color, tooltip)) in [
                (ActionIcon::Locate, None, ACCENT, "Locate this window"),
                (
                    ActionIcon::Arrange,
                    Some(WindowMode::Arranged),
                    ACCENT,
                    "Arrange in this slot",
                ),
                (
                    ActionIcon::Park,
                    Some(WindowMode::Parked),
                    PARKED,
                    "Park at bottom-right",
                ),
                (
                    ActionIcon::Ignore,
                    Some(WindowMode::Ignored),
                    egui::Color32::GRAY,
                    "Ignore this window",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let icon_rect = egui::Rect::from_min_size(
                    start + egui::vec2(index as f32 * (icon_size + gap), 0.0),
                    egui::vec2(icon_size, icon_size),
                );
                let selected = mode == Some(window.mode);
                if icon_button(ui, icon_rect, icon, selected, color, tooltip).clicked() {
                    action_clicked = true;
                    if let Some(mode) = mode {
                        self.set_window_mode(window, mode);
                    } else {
                        self.send(ControllerCommand::Locate(window.id));
                    }
                }
            }
        }

        if response.on_hover_text(window_subtitle(window)).clicked()
            && !action_clicked
            && let Some(command) =
                slot_click_command(&mut self.selected_table, Some(window.id), slot.id)
        {
            self.send(command);
        }
    }

    fn application_board(&mut self, ui: &mut egui::Ui) {
        let mut windows: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.poker_client.is_none())
            .cloned()
            .collect();
        windows.sort_by_key(|window| window_mode_rank(window.mode));

        if windows.is_empty() {
            empty_section(
                ui,
                "No other windows",
                "Ordinary application windows appear here.",
            );
            return;
        }

        let content_width = ui.available_width();
        let tile_width = ((content_width - 4.0) / 2.0).max(82.0);
        let row_count = windows.len().div_ceil(2);
        for (row_index, row) in windows.chunks(2).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for window in row {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, APPLICATION_TILE_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.application_tile(ui, window),
                    );
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(CARD_ROW_GAP);
            }
        }
    }

    fn application_tile(&mut self, ui: &mut egui::Ui, window: &CandidateView) -> egui::Response {
        let card_fill = if ui.visuals().dark_mode {
            egui::Color32::from_gray(27)
        } else {
            egui::Color32::from_gray(248)
        };
        let card_stroke =
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);

        egui::Frame::new()
            .fill(card_fill)
            .stroke(card_stroke)
            .corner_radius(6)
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let text_width = (ui.available_width() - 68.0).max(24.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 34.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&window.label).strong())
                                    .truncate(),
                            )
                            .on_hover_text(&window.label);
                            ui.label(
                                egui::RichText::new(window_subtitle(window))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        },
                    );
                    self.application_window_controls(ui, window);
                });
            })
            .response
    }

    fn application_window_controls(&self, ui: &mut egui::Ui, window: &CandidateView) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (icon, mode, color, tooltip) in [
                (
                    ActionIcon::Locate,
                    None,
                    ACCENT,
                    "Locate this application window",
                ),
                (
                    ActionIcon::Ignore,
                    Some(WindowMode::Ignored),
                    egui::Color32::GRAY,
                    "Ignore: leave this application window untouched",
                ),
                (
                    ActionIcon::FillSpace,
                    Some(WindowMode::FreeSpace),
                    FREE_SPACE,
                    "Fill space: use only the vertical strip right of poker tables",
                ),
                (
                    ActionIcon::TopRight,
                    Some(WindowMode::TopRight),
                    TOP_RIGHT,
                    "Top-right: keep its size and anchor it at the display's top-right",
                ),
            ] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                if icon_button(ui, rect, icon, mode == Some(window.mode), color, tooltip).clicked()
                {
                    if let Some(mode) = mode {
                        self.set_window_mode(window, mode);
                    } else {
                        self.send(ControllerCommand::Locate(window.id));
                    }
                }
            }
        });
    }

    fn set_window_mode(&self, window: &CandidateView, mode: WindowMode) {
        if window.mode != mode {
            self.send(ControllerCommand::SetWindowMode {
                id: window.id,
                mode,
            });
        }
    }

    fn show_settings_viewport(&mut self, context: &egui::Context) {
        if !self.settings_open.load(Ordering::Acquire) {
            return;
        }

        let viewport_id = settings_viewport_id();
        let settings = Arc::clone(&self.settings);
        let settings_open = Arc::clone(&self.settings_open);
        let commands = self.commands.clone();
        let root_context = context.clone();
        let desired_height = lock_settings(&self.settings).desired_height;
        let builder = egui::ViewportBuilder::default()
            .with_title("Table Arranger Control — Settings")
            .with_inner_size([SETTINGS_WIDTH, desired_height])
            .with_min_inner_size([SETTINGS_WIDTH, SETTINGS_MIN_HEIGHT])
            .with_max_inner_size([SETTINGS_WIDTH, SETTINGS_MAX_HEIGHT])
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_icon(Arc::clone(&self.app_icon));

        context.show_viewport_deferred(viewport_id, builder, move |ui, _class| {
            if ui.input(|input| input.viewport().close_requested()) {
                settings_open.store(false, Ordering::Release);
                root_context.request_repaint_of(egui::ViewportId::ROOT);
                return;
            }

            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
            let mut state = lock_settings(&settings);
            let content = egui::Frame::new()
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| settings_controls(ui, &mut state, &commands));
            let desired_height = (content.response.rect.height() + 2.0)
                .ceil()
                .clamp(SETTINGS_MIN_HEIGHT, SETTINGS_MAX_HEIGHT);
            if (state.desired_height - desired_height).abs() > 1.0 {
                state.desired_height = desired_height;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        SETTINGS_WIDTH,
                        desired_height,
                    )));
            }

            if state.pending_hotkeys.is_some() {
                root_context.request_repaint_of(egui::ViewportId::ROOT);
            }
        });

        if self.settings_focus_requested {
            context.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Focus);
            self.settings_focus_requested = false;
        }
    }

    fn fit_panel_height(&self, context: &egui::Context) {
        let target_height = desired_panel_height(&self.snapshot);
        let current = context.input(|input| input.viewport().inner_rect);
        let current_height = current.map_or(0.0, |rect| rect.height());
        if (current_height - target_height).abs() > 1.0 {
            let width = current
                .map_or(740.0, |rect| rect.width())
                .clamp(680.0, 820.0);
            context.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                width,
                target_height,
            )));
        }
    }
}

fn lock_settings(
    settings: &Mutex<SettingsViewportState>,
) -> std::sync::MutexGuard<'_, SettingsViewportState> {
    settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("table-arranger-settings")
}

fn settings_controls(
    ui: &mut egui::Ui,
    state: &mut SettingsViewportState,
    commands: &Sender<ControllerCommand>,
) {
    let snapshot = Arc::clone(&state.snapshot);
    let active = snapshot
        .candidates
        .iter()
        .filter(|window| window.mode == WindowMode::Arranged)
        .count();
    let parked = snapshot
        .candidates
        .iter()
        .filter(|window| window.mode == WindowMode::Parked)
        .count();
    let positioned = snapshot
        .candidates
        .iter()
        .filter(|window| matches!(window.mode, WindowMode::TopRight | WindowMode::FreeSpace))
        .count();
    let ignored = snapshot
        .candidates
        .len()
        .saturating_sub(active + parked + positioned);

    ui.label(egui::RichText::new("Workspace").strong());
    ui.label(
        egui::RichText::new(format!(
            "{active} active · {parked} parked · {positioned} positioned · {ignored} ignored"
        ))
        .small()
        .color(ui.visuals().weak_text_color()),
    );
    let has_failure = snapshot.tables.iter().any(|table| {
        matches!(
            table.status,
            TableStatus::AccessDenied | TableStatus::MoveFailed(_)
        )
    }) || snapshot.candidates.iter().any(|window| {
        matches!(
            window.status,
            Some(TableStatus::AccessDenied | TableStatus::MoveFailed(_))
        )
    });
    let status_color = if has_failure {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.label(
        egui::RichText::new(&snapshot.status_message)
            .small()
            .color(status_color),
    );
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Display");
        let selected_text = snapshot
            .monitors
            .iter()
            .find(|monitor| Some(&monitor.id) == snapshot.selected_monitor.as_ref())
            .map_or("Select display", |monitor| monitor.label.as_str());
        egui::ComboBox::from_id_salt("settings-target-display")
            .selected_text(selected_text)
            .width(330.0)
            .show_ui(ui, |ui| {
                for monitor in &snapshot.monitors {
                    let selected = Some(&monitor.id) == snapshot.selected_monitor.as_ref();
                    if ui.selectable_label(selected, &monitor.label).clicked() {
                        let _ = commands.send(ControllerCommand::SelectMonitor(monitor.id.clone()));
                    }
                }
            });
    });

    let mut preserve_requested = snapshot.preserve_table_slots_requested;
    if ui
        .checkbox(&mut preserve_requested, "Preserve table slots")
        .on_hover_text(if snapshot.preserve_table_slots_auto_suppressed {
            "Preference is On but currently inactive because two or more active poker columns already occupy the footprint; uncheck to keep it manually Off"
        } else {
            "Keep empty poker positions up to a maximum two-column footprint"
        })
        .changed()
    {
        let _ = commands.send(ControllerCommand::SetPreserveTableSlots(preserve_requested));
    }
    if snapshot.preserve_table_slots_auto_suppressed {
        ui.label(
            egui::RichText::new("Currently inactive: two or more active columns")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }
    ui.horizontal_wrapped(|ui| {
        for (icon, label) in [
            (ActionIcon::Locate, "Locate"),
            (ActionIcon::Arrange, "Active"),
            (ActionIcon::Park, "Park"),
            (ActionIcon::Ignore, "Ignore"),
            (ActionIcon::FillSpace, "Fill space"),
            (ActionIcon::TopRight, "Top-right"),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            let _ = icon_button(ui, rect, icon, false, ACCENT, label);
            ui.label(egui::RichText::new(label).small());
        }
    });

    ui.label("Default for new non-poker windows");
    let mut default_mode = snapshot.default_application_mode;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut default_mode, ApplicationDefault::Ignored, "Ignore");
        ui.selectable_value(&mut default_mode, ApplicationDefault::FreeSpace, "Free");
        ui.selectable_value(&mut default_mode, ApplicationDefault::TopRight, "Top");
    });
    if default_mode != snapshot.default_application_mode {
        let _ = commands.send(ControllerCommand::SetDefaultApplicationMode(default_mode));
    }
    ui.label(
        egui::RichText::new("Saved choices on individual windows take priority.")
            .small()
            .color(ui.visuals().weak_text_color()),
    );
    ui.separator();

    egui::CollapsingHeader::new("Global hotkeys")
        .id_salt("settings-global-hotkeys")
        .default_open(false)
        .show(ui, |ui| {
            shortcut_field(ui, "Arrange", &mut state.shortcut_draft.arrange_now);
            shortcut_field(
                ui,
                "Toggle focused",
                &mut state.shortcut_draft.toggle_focused,
            );
            shortcut_field(ui, "Show panel", &mut state.shortcut_draft.show_panel);
            ui.separator();
            for (index, shortcut) in state.shortcut_draft.toggle_slots.iter_mut().enumerate() {
                shortcut_field(ui, &format!("Table {}", index + 1), shortcut);
            }
            ui.add_space(3.0);
            if ui
                .add_sized(
                    [ui.available_width(), 22.0],
                    egui::Button::new(
                        egui::RichText::new("Apply hotkeys").color(egui::Color32::WHITE),
                    )
                    .fill(ACCENT)
                    .corner_radius(5),
                )
                .clicked()
            {
                state.pending_hotkeys = Some(state.shortcut_draft.clone());
            }
            for error in &state.shortcut_errors {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });

    ui.label(
        egui::RichText::new(format!(
            "Version {} · administrator mode",
            env!("CARGO_PKG_VERSION")
        ))
        .small()
        .color(ui.visuals().weak_text_color()),
    );
}

impl eframe::App for TableArrangerApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_background_events(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style())
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                self.top_bar(ui);
                ui.separator();
                self.workspace(ui);
            });
        self.show_settings_viewport(ui.ctx());
        self.fit_panel_height(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.send(ControllerCommand::Shutdown);
        info!("Table Arranger Control stopped");
    }
}

fn desired_panel_height(snapshot: &UiSnapshot) -> f32 {
    let poker = snapshot
        .candidates
        .iter()
        .filter(|window| window.poker_client.is_some())
        .count();
    let ignored_poker = snapshot
        .candidates
        .iter()
        .filter(|window| window.poker_client.is_some() && window.mode == WindowMode::Ignored)
        .count();
    let applications = snapshot
        .candidates
        .iter()
        .filter(|window| window.poker_client.is_none())
        .count();
    let poker_height = if poker > 0 || !snapshot.poker_slots.is_empty() {
        let mirror_height = snapshot.poker_work_area.map_or(MIRROR_MIN_HEIGHT, |work| {
            let aspect = work.width.max(1) as f32 / work.height.max(1) as f32;
            (MIRROR_MAX_PANE_WIDTH / aspect).max(MIRROR_MIN_HEIGHT)
        });
        mirror_height + ignored_poker.div_ceil(2) as f32 * 18.0
    } else {
        MINIMUM_BOARD_HEIGHT
    };
    let application_rows = applications.div_ceil(2);
    let application_height = application_rows as f32 * APPLICATION_TILE_HEIGHT
        + application_rows.saturating_sub(1) as f32 * CARD_ROW_GAP;
    let cards_height = poker_height
        .max(application_height)
        .max(MINIMUM_BOARD_HEIGHT);

    (PANEL_CHROME_HEIGHT + cards_height).clamp(180.0, 420.0)
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
enum ActionIcon {
    Locate,
    Arrange,
    Park,
    Ignore,
    FillSpace,
    TopRight,
}

fn project_rect(source: Rect, work_area: Rect, target: egui::Rect) -> egui::Rect {
    let scale_x = target.width() / work_area.width.max(1) as f32;
    let scale_y = target.height() / work_area.height.max(1) as f32;
    let scale = scale_x.min(scale_y);
    let projected_size = egui::vec2(
        work_area.width.max(1) as f32 * scale,
        work_area.height.max(1) as f32 * scale,
    );
    let origin = target.min + (target.size() - projected_size) * 0.5;
    egui::Rect::from_min_size(
        origin
            + egui::vec2(
                source.left.saturating_sub(work_area.left) as f32 * scale,
                source.top.saturating_sub(work_area.top) as f32 * scale,
            ),
        egui::vec2(
            source.width.max(1) as f32 * scale,
            source.height.max(1) as f32 * scale,
        ),
    )
    .intersect(target)
}

fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let dash = 4.0;
    let gap = 3.0;
    for (from, to) in [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ] {
        let vector = to - from;
        let length = vector.length();
        if length <= 0.0 {
            continue;
        }
        let direction = vector / length;
        let mut offset = 0.0;
        while offset < length {
            let end = (offset + dash).min(length);
            painter.line_segment(
                [from + direction * offset, from + direction * end],
                egui::Stroke::new(1.0, color),
            );
            offset += dash + gap;
        }
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: ActionIcon,
    selected: bool,
    color: egui::Color32,
    tooltip: &str,
) -> egui::Response {
    let response = ui
        .interact(
            rect,
            ui.make_persistent_id((
                "poker-action",
                icon,
                rect.min.x.to_bits(),
                rect.min.y.to_bits(),
            )),
            egui::Sense::click(),
        )
        .on_hover_text(tooltip);
    let fill = if selected {
        color
    } else if response.hovered() {
        color.gamma_multiply(0.35)
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    ui.painter().rect_filled(rect, 3.0, fill);
    let ink = if selected {
        egui::Color32::WHITE
    } else {
        ui.visuals().text_color()
    };
    let stroke = egui::Stroke::new(1.2, ink);
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.24;
    match icon {
        ActionIcon::Locate => {
            ui.painter().circle_stroke(center, radius, stroke);
            ui.painter().line_segment(
                [
                    egui::pos2(center.x, rect.top() + 2.0),
                    egui::pos2(center.x, center.y - radius),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 2.0, center.y),
                    egui::pos2(center.x - radius, center.y),
                ],
                stroke,
            );
        }
        ActionIcon::Arrange => {
            let size = radius * 0.72;
            for offset in [
                egui::vec2(-size, -size),
                egui::vec2(size, -size),
                egui::vec2(-size, size),
                egui::vec2(size, size),
            ] {
                ui.painter().rect_stroke(
                    egui::Rect::from_center_size(center + offset, egui::vec2(size, size)),
                    0.5,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        ActionIcon::Park => {
            ui.painter().line_segment(
                [
                    egui::pos2(center.x, rect.top() + 3.0),
                    egui::pos2(center.x, rect.bottom() - 5.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(center.x - 3.0, rect.bottom() - 8.0),
                    egui::pos2(center.x, rect.bottom() - 5.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(center.x + 3.0, rect.bottom() - 8.0),
                    egui::pos2(center.x, rect.bottom() - 5.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 3.0, rect.bottom() - 3.0),
                    egui::pos2(rect.right() - 3.0, rect.bottom() - 3.0),
                ],
                stroke,
            );
        }
        ActionIcon::Ignore => {
            ui.painter().circle_stroke(center, radius, stroke);
            ui.painter().line_segment(
                [
                    egui::pos2(center.x - radius - 2.0, center.y + radius + 2.0),
                    egui::pos2(center.x + radius + 2.0, center.y - radius - 2.0),
                ],
                stroke,
            );
        }
        ActionIcon::FillSpace => {
            let bounds = rect.shrink(3.0);
            ui.painter()
                .rect_stroke(bounds, 0.5, stroke, egui::StrokeKind::Inside);
            ui.painter().line_segment(
                [
                    egui::pos2(bounds.left() + 2.0, center.y),
                    egui::pos2(bounds.right() - 2.0, center.y),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(bounds.left() + 2.0, center.y),
                    egui::pos2(bounds.left() + 4.0, center.y - 2.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(bounds.right() - 2.0, center.y),
                    egui::pos2(bounds.right() - 4.0, center.y + 2.0),
                ],
                stroke,
            );
        }
        ActionIcon::TopRight => {
            let start = egui::pos2(rect.left() + 4.0, rect.bottom() - 4.0);
            let end = egui::pos2(rect.right() - 3.0, rect.top() + 3.0);
            ui.painter().line_segment([start, end], stroke);
            ui.painter()
                .line_segment([end, egui::pos2(end.x - 4.0, end.y)], stroke);
            ui.painter()
                .line_segment([end, egui::pos2(end.x, end.y + 4.0)], stroke);
        }
    }
    response
}

fn short_label(label: &str, max_chars: usize) -> String {
    let mut chars = label.chars();
    let short: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

fn placeholder_number(slot: PokerSlotId) -> usize {
    slot.column * 2 + slot.row.unwrap_or(0) as usize + 1
}

fn slot_click_command(
    selected: &mut Option<WindowId>,
    occupant: Option<WindowId>,
    destination: PokerSlotId,
) -> Option<ControllerCommand> {
    match (selected.take(), occupant) {
        (None, Some(window)) => {
            *selected = Some(window);
            None
        }
        (Some(source), Some(window)) if source == window => None,
        (Some(source), _) => Some(ControllerCommand::MoveToSlot {
            source,
            destination,
        }),
        (None, None) => None,
    }
}

fn empty_section(ui: &mut egui::Ui, title: &str, detail: &str) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(title).strong());
            ui.label(
                egui::RichText::new(detail)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
}

fn window_subtitle(window: &CandidateView) -> String {
    match window.mode {
        WindowMode::Arranged => format!(
            "Table {} · {}",
            window.slot.unwrap_or_default(),
            window
                .status
                .as_ref()
                .map_or_else(|| "Ready".to_owned(), ToString::to_string)
        ),
        WindowMode::Parked => "Parked at bottom-right".to_owned(),
        WindowMode::TopRight => format!("Top-right · {}", status_text(window)),
        WindowMode::FreeSpace => format!("Fills right-side space · {}", status_text(window)),
        WindowMode::Ignored if window.likely_table => "Not managed · likely table".to_owned(),
        WindowMode::Ignored => format!("Ignored · {}", window.process_name),
    }
}

fn status_text(window: &CandidateView) -> String {
    window
        .status
        .as_ref()
        .map_or_else(|| "Ready".to_owned(), ToString::to_string)
}

const fn window_mode_rank(mode: WindowMode) -> u8 {
    match mode {
        WindowMode::Arranged => 0,
        WindowMode::TopRight | WindowMode::FreeSpace => 1,
        WindowMode::Parked => 2,
        WindowMode::Ignored => 3,
    }
}

fn shortcut_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_sized([170.0, 20.0], egui::TextEdit::singleline(value));
        });
    });
}

fn configure_style(context: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*context.style_of(theme)).clone();
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(9.0));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(10.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.0));
        style.spacing.item_spacing = egui::vec2(4.0, 4.0);
        style.spacing.button_padding = egui::vec2(5.0, 3.0);
        style.spacing.interact_size = egui::vec2(24.0, 20.0);
        style.visuals.window_corner_radius = egui::CornerRadius::same(7);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.open.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.visuals.selection.bg_fill = ACCENT;
        context.set_style_of(theme, style);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    use crossbeam_channel::unbounded;
    use eframe::egui;

    use super::{
        APPLICATION_TILE_HEIGHT, SETTINGS_MAX_HEIGHT, SettingsViewportState, TableArrangerApp,
        settings_controls, slot_click_command, window_mode_rank,
    };
    use crate::{
        config::HotkeySettings,
        controller::ControllerCommand,
        model::{
            CandidateView, PokerClientKind, PokerSlotId, PokerSlotView, Rect, UiSnapshot, WindowId,
            WindowMode,
        },
        tray::TrayAction,
    };

    #[test]
    fn window_groups_are_arranged_then_parked_then_ignored() {
        assert!(window_mode_rank(WindowMode::Arranged) < window_mode_rank(WindowMode::TopRight));
        assert_eq!(
            window_mode_rank(WindowMode::TopRight),
            window_mode_rank(WindowMode::FreeSpace)
        );
        assert!(window_mode_rank(WindowMode::FreeSpace) < window_mode_rank(WindowMode::Parked));
        assert!(window_mode_rank(WindowMode::Parked) < window_mode_rank(WindowMode::Ignored));
    }

    #[test]
    fn mirror_projection_preserves_desktop_geometry() {
        let work = Rect::new(100, 50, 2000, 1000);
        let target = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(400.0, 200.0));
        let projected = super::project_rect(Rect::new(1100, 50, 1000, 500), work, target);

        assert_eq!(projected.min, egui::pos2(210.0, 20.0));
        assert_eq!(projected.size(), egui::vec2(200.0, 100.0));
    }

    #[test]
    fn selected_table_clicking_placeholder_two_emits_a_move_command() {
        let mut selected = Some(WindowId(1));
        let destination = PokerSlotId::club(0, 1);

        let command = slot_click_command(&mut selected, None, destination);
        assert!(matches!(
            command,
            Some(ControllerCommand::MoveToSlot {
                source: WindowId(1),
                destination: received,
            }) if received == destination
        ));
        assert_eq!(selected, None);
    }

    #[test]
    fn workspace_content_stays_inside_a_compact_panel() {
        let (commands, _command_rx) = unbounded::<ControllerCommand>();
        let (_snapshot_tx, snapshots) = unbounded::<Arc<UiSnapshot>>();
        let (_hotkey_tx, hotkey_events) = unbounded::<u32>();
        let (_tray_tx, tray_events) = unbounded::<TrayAction>();
        let snapshot = UiSnapshot {
            candidates: (0..16)
                .map(|index| CandidateView {
                    id: WindowId(index + 1),
                    label: format!("Window with a deliberately long title {index}"),
                    process_name: if index < 8 {
                        "ClubGG.exe".to_owned()
                    } else {
                        "application.exe".to_owned()
                    },
                    class_name: "TestWindow".to_owned(),
                    poker_client: (index < 8).then_some(PokerClientKind::ClubGg),
                    current_rect: Rect::new(0, 0, 640, 480),
                    likely_table: index < 8,
                    mode: if index == 0 {
                        WindowMode::Arranged
                    } else if index < 8 {
                        WindowMode::Ignored
                    } else {
                        WindowMode::FreeSpace
                    },
                    slot: (index == 0).then_some(1),
                    status: None,
                })
                .collect(),
            poker_work_area: Some(Rect::new(0, 0, 1920, 1040)),
            poker_slots: vec![PokerSlotView {
                id: PokerSlotId::club(0, 0),
                rect: Rect::new(0, 0, 640, 480),
                occupant: Some(WindowId(1)),
                parked: false,
            }],
            ..UiSnapshot::default()
        };
        let snapshot = Arc::new(snapshot);
        let mut app = TableArrangerApp {
            commands,
            snapshots,
            snapshot: Arc::clone(&snapshot),
            hotkeys: None,
            hotkey_events,
            _tray: None,
            tray_events,
            settings: Arc::new(Mutex::new(SettingsViewportState {
                snapshot,
                shortcut_draft: HotkeySettings::default(),
                shortcut_errors: Vec::new(),
                pending_hotkeys: None,
                desired_height: 300.0,
            })),
            settings_open: Arc::new(AtomicBool::new(false)),
            settings_focus_requested: false,
            app_icon: Arc::new(super::egui_icon()),
            selected_table: None,
            exiting: false,
        };

        egui::__run_test_ui(|ui| {
            super::configure_style(ui.ctx());
            ui.set_style((*ui.ctx().style_of(egui::Theme::Dark)).clone());
            ui.set_width(166.0);
            ui.set_height(APPLICATION_TILE_HEIGHT);
            let bounds = ui.max_rect();
            let first_application = app.snapshot.candidates[8].clone();
            let card = app.application_tile(ui, &first_application);
            assert!(
                card.rect.height() <= APPLICATION_TILE_HEIGHT,
                "application tile height {} exceeded allocation {}",
                card.rect.height(),
                APPLICATION_TILE_HEIGHT
            );
            assert!(
                card.rect.bottom() <= bounds.bottom() + 1.0,
                "application tile bottom {} exceeded bound {}",
                card.rect.bottom(),
                bounds.bottom()
            );
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(320.0);
            ui.set_height(280.0);
            let bounds = ui.max_rect();
            app.poker_board(ui);
            assert!(
                ui.min_rect().right() <= bounds.right() + 1.0,
                "poker board right {} exceeded bound {}",
                ui.min_rect().right(),
                bounds.right()
            );
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "poker board bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(280.0);
            ui.set_height(280.0);
            let bounds = ui.max_rect();
            app.application_board(ui);
            assert!(
                ui.min_rect().right() <= bounds.right() + 1.0,
                "application board right {} exceeded bound {}",
                ui.min_rect().right(),
                bounds.right()
            );
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "application board bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(600.0);
            ui.set_height(280.0);
            let bounds = ui.max_rect();
            app.workspace(ui);
            assert!(
                ui.min_rect().right() <= bounds.right() + 1.0,
                "workspace right {} exceeded bound {}",
                ui.min_rect().right(),
                bounds.right()
            );
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "workspace bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(740.0);
            ui.set_height(super::desired_panel_height(&app.snapshot));
            let bounds = ui.max_rect();
            egui::Frame::central_panel(ui.style())
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    app.top_bar(ui);
                    ui.separator();
                    app.workspace(ui);
                });
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "fitted main panel bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
        assert!((356.0..=357.0).contains(&super::desired_panel_height(&app.snapshot)));
        assert_eq!(super::desired_panel_height(&UiSnapshot::default()), 180.0);
    }

    #[test]
    fn fitted_height_keeps_final_poker_and_application_rows_visible() {
        let snapshot = UiSnapshot {
            candidates: (0..10)
                .map(|index| CandidateView {
                    id: WindowId(index + 1),
                    label: format!("Window {index}"),
                    process_name: if index < 5 {
                        "ClubGG.exe".to_owned()
                    } else {
                        "application.exe".to_owned()
                    },
                    class_name: "TestWindow".to_owned(),
                    poker_client: (index < 5).then_some(PokerClientKind::ClubGg),
                    current_rect: Rect::new(0, 0, 640, 480),
                    likely_table: index < 5,
                    mode: if index < 2 {
                        WindowMode::Arranged
                    } else if index < 5 {
                        WindowMode::Parked
                    } else {
                        WindowMode::TopRight
                    },
                    slot: (index < 2).then_some(index as usize + 1),
                    status: None,
                })
                .collect(),
            ..UiSnapshot::default()
        };

        assert_eq!(super::desired_panel_height(&snapshot), 224.0);
    }

    #[test]
    fn expanded_settings_fit_the_independent_viewport_without_scrolling() {
        let (commands, _command_rx) = unbounded::<ControllerCommand>();
        let mut state = SettingsViewportState {
            snapshot: Arc::new(UiSnapshot::default()),
            shortcut_draft: HotkeySettings::default(),
            shortcut_errors: Vec::new(),
            pending_hotkeys: None,
            desired_height: 300.0,
        };

        egui::__run_test_ui(|ui| {
            ui.set_width(super::SETTINGS_WIDTH - 16.0);
            ui.set_height(SETTINGS_MAX_HEIGHT);
            let header_id = ui.make_persistent_id("settings-global-hotkeys");
            let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                header_id,
                false,
            );
            header.set_open(true);
            header.store(ui.ctx());

            let bounds = ui.max_rect();
            settings_controls(ui, &mut state, &commands);
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "expanded settings bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
    }

    #[test]
    fn top_bar_never_consumes_the_workspace_height() {
        let (commands, _command_rx) = unbounded::<ControllerCommand>();
        let (_snapshot_tx, snapshots) = unbounded::<Arc<UiSnapshot>>();
        let (_hotkey_tx, hotkey_events) = unbounded::<u32>();
        let (_tray_tx, tray_events) = unbounded::<TrayAction>();
        let snapshot = Arc::new(UiSnapshot::default());
        let mut app = TableArrangerApp {
            commands,
            snapshots,
            snapshot: Arc::clone(&snapshot),
            hotkeys: None,
            hotkey_events,
            _tray: None,
            tray_events,
            settings: Arc::new(Mutex::new(SettingsViewportState {
                snapshot,
                shortcut_draft: HotkeySettings::default(),
                shortcut_errors: Vec::new(),
                pending_hotkeys: None,
                desired_height: 300.0,
            })),
            settings_open: Arc::new(AtomicBool::new(false)),
            settings_focus_requested: false,
            app_icon: Arc::new(super::egui_icon()),
            selected_table: None,
            exiting: false,
        };

        egui::__run_test_ui(|ui| {
            ui.set_width(740.0);
            ui.set_height(380.0);
            let response = app.top_bar(ui);
            assert!(
                response.rect.height() <= 44.0,
                "top bar consumed {} points and displaced the workspace",
                response.rect.height()
            );
        });
    }
}
