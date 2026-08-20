use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    rng_overlay::RngOverlay,
    tray::{TrayAction, TrayService, egui_icon},
    win32::{
        Win32Backend, acquire_single_instance, activate_existing_panel, apply_process_mitigations,
        move_panel_to_cursor, panel_is_visible, spawn_window_event_watcher, window_icon,
    },
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(22, 163, 74);
const PARKED: egui::Color32 = egui::Color32::from_rgb(217, 119, 6);
const DARK_ORANGE: egui::Color32 = egui::Color32::from_rgb(154, 77, 0);
const TOP_RIGHT: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);
const FREE_SPACE: egui::Color32 = egui::Color32::from_rgb(124, 58, 237);
const BACKGROUND_EVENT_QUEUE_CAPACITY: usize = 16;
const MIRROR_MIN_HEIGHT: f32 = 118.0;
const WINDOW_CHIP_HEIGHT: f32 = 28.0;
const WINDOW_DOCK_CHROME_HEIGHT: f32 = 26.0;
const PLACEHOLDER_BADGE: egui::Color32 = egui::Color32::from_rgb(70, 94, 78);
const PLACEHOLDER_LABEL: &str = "Placeholders";
const CARD_ROW_GAP: f32 = 4.0;
const MINIMUM_BOARD_HEIGHT: f32 = 62.0;
const PANEL_CHROME_HEIGHT: f32 = 50.0;
const PANEL_MIN_HEIGHT: f32 = 180.0;
const PANEL_MAX_HEIGHT: f32 = 620.0;
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
            .with_inner_size([740.0, 480.0])
            .with_min_inner_size([680.0, PANEL_MIN_HEIGHT])
            .with_max_inner_size([820.0, PANEL_MAX_HEIGHT])
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
    rng_overlay: RngOverlay,
    selected_table: Option<crate::model::WindowId>,
    application_icons: HashMap<WindowId, Option<egui::TextureHandle>>,
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
            rng_overlay: RngOverlay::new(),
            selected_table: None,
            application_icons: HashMap::new(),
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

    fn toggle_panel(context: &egui::Context) {
        if panel_is_visible() {
            Self::hide_panel(context);
        } else {
            let _ = move_panel_to_cursor();
            Self::show_panel(context);
        }
    }

    fn handle_background_events(&mut self, context: &egui::Context) {
        for snapshot in self.snapshots.try_iter() {
            self.application_icons.retain(|id, _| {
                snapshot
                    .candidates
                    .iter()
                    .any(|candidate| candidate.id == *id)
            });
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
                HotkeyAction::TogglePanel => Self::toggle_panel(context),
                HotkeyAction::LocateClubGgLobbies => {
                    self.send(ControllerCommand::LocateClubGgLobbies);
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
        let parked_tables: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| {
                window.poker_client.is_some()
                    && !window.is_clubgg_lobby
                    && window.mode == WindowMode::Parked
            })
            .cloned()
            .collect();
        ui.horizontal_wrapped(|ui| {
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
            let preserve_requested = self.snapshot.preserve_table_slots_requested;
            if ui
                .add_sized(
                    [64.0, 24.0],
                    egui::Button::new("Space").selected(preserve_requested),
                )
                .on_hover_text(if self.snapshot.preserve_table_slots_auto_suppressed {
                    "Preserve table slots is On but currently inactive because two or more active poker columns already occupy the footprint"
                } else if preserve_requested {
                    "Preserve table slots is On"
                } else {
                    "Preserve table slots is Off"
                })
                .clicked()
            {
                self.send(ControllerCommand::SetPreserveTableSlots(
                    !preserve_requested,
                ));
            }
            let mut automatic = self.snapshot.auto_arrange;
            if ui
                .add_sized(
                    [64.0, 24.0],
                    egui::Button::new("Auto").selected(automatic),
                )
                .on_hover_text("Automatically reapply the workspace after window changes")
                .clicked()
            {
                automatic = !automatic;
                self.send(ControllerCommand::SetAutoArrange(automatic));
            }
            let rng_enabled = self.rng_overlay.enabled();
            if ui
                .add_sized(
                    [64.0, 24.0],
                    egui::Button::new("RnG").selected(rng_enabled),
                )
                .on_hover_text("Show or hide the 1–100 random number overlay")
                .clicked()
            {
                self.rng_overlay
                    .toggle(ui.ctx(), selected_work_area(&self.snapshot));
            }
            if ui
                .add_sized([64.0, 24.0], egui::Button::new("Settings"))
                .on_hover_text("Workspace, display, defaults, and hotkeys")
                .clicked()
            {
                self.settings_open.store(true, Ordering::Release);
                self.settings_focus_requested = true;
            }
            let lobby_count = self
                .snapshot
                .candidates
                .iter()
                .filter(|window| window.is_clubgg_lobby)
                .count();
            if ui
                .add_enabled(
                    lobby_count > 0,
                    egui::Button::new(
                        egui::RichText::new("GGLobby").color(egui::Color32::WHITE),
                    )
                    .fill(DARK_ORANGE)
                    .min_size(egui::vec2(64.0, 24.0)),
                )
                .on_hover_text(if lobby_count == 0 {
                    "No ClubGG lobbies detected".to_owned()
                } else {
                    format!(
                        "Locate {} parked ClubGG lobb{} one by one without moving them",
                        lobby_count,
                        if lobby_count == 1 { "y" } else { "ies" }
                    )
                })
                .clicked()
            {
                self.send(ControllerCommand::LocateClubGgLobbies);
            }
            for window in &parked_tables {
                let response = self.parked_table_button(ui, window);
                if response.clicked_by(egui::PointerButton::Primary) {
                    self.send(ControllerCommand::Locate(window.id));
                } else if response.clicked_by(egui::PointerButton::Secondary) {
                    self.set_window_mode(window, WindowMode::Arranged);
                }
            }
            if ui
                .add_sized([26.0, 24.0], egui::Button::new("-"))
                .on_hover_text("Hide to tray")
                .clicked()
            {
                Self::hide_panel(ui.ctx());
            }
        })
        .response
    }

    fn parked_table_button(&mut self, ui: &mut egui::Ui, window: &CandidateView) -> egui::Response {
        let texture = self.application_icon_texture(ui.ctx(), window.id);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        let response = response.on_hover_text(format!(
            "{}\nLeft click: Locate\nRight click: Unpark",
            window.label
        ));
        let fill = if response.hovered() {
            PARKED.gamma_multiply(0.55)
        } else {
            PARKED.gamma_multiply(0.28)
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        ui.painter().rect_stroke(
            rect,
            5.0,
            egui::Stroke::new(1.0, PARKED),
            egui::StrokeKind::Inside,
        );
        let icon_rect = rect.shrink(3.0);
        if let Some(texture) = texture {
            ui.painter().image(
                texture,
                icon_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            let fallback = match window.poker_client {
                Some(PokerClientKind::LdPlayer) => "P2",
                _ => "CG",
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                fallback,
                egui::FontId::proportional(8.0),
                ui.visuals().text_color(),
            );
        }
        response
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

        self.poker_board(ui);
        self.window_dock(ui);
    }

    fn poker_board(&mut self, ui: &mut egui::Ui) {
        let windows: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.poker_client.is_some() && !window.is_clubgg_lobby)
            .cloned()
            .collect();
        let mut applications: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.poker_client.is_none() && !window.is_clubgg_lobby)
            .cloned()
            .collect();
        applications.sort_by_key(|window| window_mode_rank(window.mode));

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
        let board_height = (board_width / aspect).max(MIRROR_MIN_HEIGHT);
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
        let mut occupied_right = board_rect.left();
        for slot in &slots {
            let rect = project_rect(slot.rect, work_area, board_rect);
            occupied_right = occupied_right.max(rect.right());
            self.mirrored_slot(ui, rect, slot, &windows);
        }

        let application_area = egui::Rect::from_min_max(
            egui::pos2(
                (occupied_right + 2.0).min(board_rect.right()),
                board_rect.top(),
            ),
            board_rect.max,
        );
        self.application_overlay(ui, application_area, &applications);

        if self
            .selected_table
            .is_some_and(|selected| !windows.iter().any(|window| window.id == selected))
        {
            self.selected_table = None;
        }
    }

    fn application_overlay(
        &mut self,
        ui: &mut egui::Ui,
        board_rect: egui::Rect,
        applications: &[CandidateView],
    ) {
        for (window, rect) in applications
            .iter()
            .zip(application_overlay_rects(board_rect, applications.len()))
        {
            self.application_mirror_tile(ui, rect, window);
        }
    }

    fn application_mirror_tile(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        window: &CandidateView,
    ) {
        let fill = if ui.visuals().dark_mode {
            egui::Color32::from_gray(27)
        } else {
            egui::Color32::from_gray(248)
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );

        if let Some(texture) = self.application_icon_texture(ui.ctx(), window.id) {
            let available_height = (rect.height() - 34.0).max(0.0);
            let app_icon_size = (rect.width() * 0.34)
                .min(available_height * 0.48)
                .clamp(18.0, 48.0);
            let app_icon_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(app_icon_size, app_icon_size),
            );
            ui.painter().image(
                texture,
                app_icon_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        let icon_size = ((rect.width() - 6.0) / 3.0).clamp(8.0, 12.0);
        let gap = 1.0;
        let total = icon_size * 3.0 + gap * 2.0;
        let start = egui::pos2(
            rect.center().x - total / 2.0,
            rect.bottom() - icon_size - 2.0,
        );
        let mut control_rects = Vec::with_capacity(3);
        for (index, (icon, mode, color, tooltip)) in [
            (
                ActionIcon::Ignore,
                WindowMode::Ignored,
                egui::Color32::GRAY,
                "Ignore: leave this application window untouched",
            ),
            (
                ActionIcon::FillSpace,
                WindowMode::FreeSpace,
                FREE_SPACE,
                "Fill space: use only the vertical strip right of poker tables",
            ),
            (
                ActionIcon::TopRight,
                WindowMode::TopRight,
                TOP_RIGHT,
                "Top-right: keep its size and anchor it at the display's top-right",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let control = egui::Rect::from_min_size(
                start + egui::vec2(index as f32 * (icon_size + gap), 0.0),
                egui::vec2(icon_size, icon_size),
            );
            control_rects.push(control);
            if icon_button(ui, control, icon, mode == window.mode, color, tooltip).clicked() {
                self.set_window_mode(window, mode);
            }
        }

        let label_chars = ((rect.width() / 7.0).floor() as usize).clamp(4, 18);
        ui.painter().text(
            egui::pos2(rect.center().x, rect.top() + 3.0),
            egui::Align2::CENTER_TOP,
            short_label(&window.label, label_chars),
            egui::FontId::proportional(8.5),
            ui.visuals().text_color(),
        );
        let locate_clicked = clickable_body_rects(rect, &control_rects)
            .into_iter()
            .enumerate()
            .any(|(index, body)| {
                ui.interact(
                    body,
                    ui.make_persistent_id(("application-mirror-body", window.id.0, index)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!(
                    "{}\n{}\nLeft click: Locate",
                    window.label,
                    window_subtitle(window)
                ))
                .clicked()
            });
        if locate_clicked {
            self.send(ControllerCommand::Locate(window.id));
        }
    }

    fn application_icon_texture(
        &mut self,
        context: &egui::Context,
        id: WindowId,
    ) -> Option<egui::TextureId> {
        self.application_icons
            .entry(id)
            .or_insert_with(|| {
                window_icon(id).map(|icon| {
                    context.load_texture(
                        format!("application-icon-{}", id.0),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [icon.size, icon.size],
                            &icon.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                })
            })
            .as_ref()
            .map(egui::TextureHandle::id)
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
        let Some(window) = occupant else {
            let response = ui.interact(
                rect,
                ui.make_persistent_id(("poker-slot", slot.id.column, slot.id.row)),
                egui::Sense::click(),
            );
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
        let badge_hit_rect = badge_rect.expand(3.0).intersect(rect);
        let badge_response = ui
            .interact(
                badge_hit_rect,
                ui.make_persistent_id(("table-number", window.id.0)),
                egui::Sense::click(),
            )
            .on_hover_text("Select this table number for swapping");
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

        let icon_size = ((rect.width() - 6.0) / 3.0).clamp(9.0, 16.0);
        let gap = 1.0;
        let total = icon_size * 3.0 + gap * 2.0;
        let action_top = rect.bottom() - icon_size - 2.0;
        let mut control_rects = vec![badge_hit_rect];
        if rect.width() >= total + 4.0 && rect.height() >= icon_size * 2.0 + 5.0 {
            let start = egui::pos2(rect.center().x - total / 2.0, action_top);
            for (index, (icon, mode, color, tooltip)) in [
                (
                    ActionIcon::Arrange,
                    WindowMode::Arranged,
                    ACCENT,
                    "Arrange in this slot",
                ),
                (
                    ActionIcon::Park,
                    WindowMode::Parked,
                    PARKED,
                    "Park at top-right",
                ),
                (
                    ActionIcon::Ignore,
                    WindowMode::Ignored,
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
                control_rects.push(icon_rect);
                let selected = mode == window.mode;
                if icon_button(ui, icon_rect, icon, selected, color, tooltip).clicked() {
                    self.set_window_mode(window, mode);
                }
            }
        }

        let body_clicked = clickable_body_rects(rect, &control_rects)
            .into_iter()
            .enumerate()
            .filter(|(_, body_rect)| body_rect.is_positive())
            .any(|(index, body_rect)| {
                ui.interact(
                    body_rect,
                    ui.make_persistent_id(("table-body", window.id.0, index)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!(
                    "{}\nLeft click: Locate\nClick the number badge to select for swapping",
                    window_subtitle(window)
                ))
                .clicked()
            });
        if badge_response.clicked() {
            if let Some(command) =
                slot_click_command(&mut self.selected_table, Some(window.id), slot.id)
            {
                self.send(command);
            }
        } else if body_clicked {
            self.send(ControllerCommand::Locate(window.id));
        }
    }

    fn window_dock(&mut self, ui: &mut egui::Ui) {
        let ignored_poker: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| {
                window.poker_client.is_some()
                    && !window.is_clubgg_lobby
                    && window.mode == WindowMode::Ignored
            })
            .cloned()
            .collect();
        if ignored_poker.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(2.0);
        self.window_dock_grid(ui, &ignored_poker);
    }

    fn window_dock_grid(&mut self, ui: &mut egui::Ui, windows: &[CandidateView]) {
        let content_width = ui.available_width();
        let columns = window_dock_columns(content_width);
        let total_gap = CARD_ROW_GAP * (columns - 1) as f32;
        let tile_width = ((content_width - total_gap) / columns as f32).max(80.0);
        let row_count = windows.len().div_ceil(columns);
        for (row_index, row) in windows.chunks(columns).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_ROW_GAP;
                for window in row {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, WINDOW_CHIP_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.ignored_poker_chip(ui, window),
                    );
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(CARD_ROW_GAP);
            }
        }
    }

    fn ignored_poker_chip(&mut self, ui: &mut egui::Ui, window: &CandidateView) {
        let mut control_rects = Vec::new();
        let card = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(egui::Stroke::new(
                1.0,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(6)
            .inner_margin(egui::Margin::same(3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let text_width = (ui.available_width() - 40.0).max(18.0);
                    ui.add_sized(
                        [text_width, 16.0],
                        egui::Label::new(short_label(&window.label, 13)).truncate(),
                    )
                    .on_hover_text(&window.label);
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (icon, mode, color, tooltip) in [
                        (
                            ActionIcon::Arrange,
                            WindowMode::Arranged,
                            ACCENT,
                            "Arrange this table",
                        ),
                        (
                            ActionIcon::Park,
                            WindowMode::Parked,
                            PARKED,
                            "Park this table",
                        ),
                        (
                            ActionIcon::Ignore,
                            WindowMode::Ignored,
                            egui::Color32::GRAY,
                            "Ignore this table",
                        ),
                    ] {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        control_rects.push(rect);
                        if icon_button(ui, rect, icon, mode == window.mode, color, tooltip)
                            .clicked()
                        {
                            self.set_window_mode(window, mode);
                        }
                    }
                });
            })
            .response;
        let locate_clicked = clickable_body_rects(card.rect, &control_rects)
            .into_iter()
            .enumerate()
            .any(|(index, rect)| {
                ui.interact(
                    rect,
                    ui.make_persistent_id(("ignored-poker-body", window.id.0, index)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!("{}\nLeft click: Locate", window.label))
                .clicked()
            });
        if locate_clicked {
            self.send(ControllerCommand::Locate(window.id));
        }
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
        let current = context.input(|input| input.viewport().inner_rect);
        let current_height = current.map_or(0.0, |rect| rect.height());
        let width = current
            .map_or(740.0, |rect| rect.width())
            .clamp(680.0, 820.0);
        let target_height = desired_panel_height(&self.snapshot, width);
        if (current_height - target_height).abs() > 1.0 {
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

    ui.horizontal_wrapped(|ui| {
        for (icon, label) in [
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
        ui.label(egui::RichText::new("Left click card = Locate").small());
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
            shortcut_field(ui, "Show/hide panel", &mut state.shortcut_draft.show_panel);
            shortcut_field(
                ui,
                "GGLobby",
                &mut state.shortcut_draft.locate_clubgg_lobbies,
            );
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
        self.rng_overlay
            .sync_work_area(selected_work_area(&self.snapshot));
        self.fit_panel_height(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.rng_overlay.stop();
        self.send(ControllerCommand::Shutdown);
        info!("Table Arranger Control stopped");
    }
}

fn selected_work_area(snapshot: &UiSnapshot) -> Rect {
    snapshot
        .selected_monitor
        .as_ref()
        .and_then(|id| snapshot.monitors.iter().find(|monitor| &monitor.id == id))
        .or_else(|| snapshot.monitors.iter().find(|monitor| monitor.primary))
        .or_else(|| snapshot.monitors.first())
        .map_or_else(|| Rect::new(0, 0, 1920, 1080), |monitor| monitor.work_area)
}

fn desired_panel_height(snapshot: &UiSnapshot, panel_width: f32) -> f32 {
    if snapshot.candidates.is_empty() && snapshot.poker_slots.is_empty() {
        return PANEL_MIN_HEIGHT;
    }

    let ignored_poker = snapshot
        .candidates
        .iter()
        .filter(|window| {
            window.poker_client.is_some()
                && !window.is_clubgg_lobby
                && window.mode == WindowMode::Ignored
        })
        .count();
    let content_width = (panel_width - 12.0).max(1.0);
    let parked_poker = snapshot
        .candidates
        .iter()
        .filter(|window| {
            window.poker_client.is_some()
                && !window.is_clubgg_lobby
                && window.mode == WindowMode::Parked
        })
        .count();
    let toolbar_width = 434.0 + parked_poker as f32 * 28.0;
    let toolbar_rows = (toolbar_width / content_width).ceil().max(1.0);
    let wrapped_toolbar_height = (toolbar_rows - 1.0) * 28.0;
    let mirror_height = snapshot
        .poker_work_area
        .map_or(MINIMUM_BOARD_HEIGHT, |work| {
            let aspect = work.width.max(1) as f32 / work.height.max(1) as f32;
            (content_width / aspect).max(MIRROR_MIN_HEIGHT)
        });

    let dock_items = ignored_poker;
    let dock_height = if dock_items == 0 {
        0.0
    } else {
        let rows = dock_items.div_ceil(window_dock_columns(content_width));
        WINDOW_DOCK_CHROME_HEIGHT
            + rows as f32 * WINDOW_CHIP_HEIGHT
            + rows.saturating_sub(1) as f32 * CARD_ROW_GAP
    };

    (PANEL_CHROME_HEIGHT + wrapped_toolbar_height + mirror_height + dock_height)
        .clamp(PANEL_MIN_HEIGHT, PANEL_MAX_HEIGHT)
}

fn window_dock_columns(content_width: f32) -> usize {
    if content_width >= 780.0 {
        8
    } else if content_width >= 710.0 {
        7
    } else {
        6
    }
}

fn application_overlay_rects(area: egui::Rect, count: usize) -> Vec<egui::Rect> {
    if count == 0 || !area.is_positive() {
        return Vec::new();
    }
    let rows = match count {
        1 => 1,
        2..=6 => 2,
        _ => (count as f32).sqrt().ceil() as usize,
    };
    let columns = count.div_ceil(rows);
    let gap = 2.0;
    let grid = area.shrink(2.0);
    if !grid.is_positive() {
        return Vec::new();
    }
    let cell_width =
        ((grid.width() - gap * columns.saturating_sub(1) as f32) / columns as f32).max(1.0);
    let cell_height =
        ((grid.height() - gap * rows.saturating_sub(1) as f32) / rows as f32).max(1.0);

    (0..count)
        .map(|index| {
            let column = index / rows;
            let row = index % rows;
            egui::Rect::from_min_size(
                grid.min
                    + egui::vec2(
                        column as f32 * (cell_width + gap),
                        row as f32 * (cell_height + gap),
                    ),
                egui::vec2(cell_width, cell_height),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
enum ActionIcon {
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

fn clickable_body_rects(tile: egui::Rect, controls: &[egui::Rect]) -> Vec<egui::Rect> {
    let mut regions = vec![tile];
    for control in controls {
        let mut next = Vec::new();
        for region in regions {
            let overlap = region.intersect(*control);
            if !overlap.is_positive() {
                next.push(region);
                continue;
            }
            for remainder in [
                egui::Rect::from_min_max(region.min, egui::pos2(region.right(), overlap.top())),
                egui::Rect::from_min_max(egui::pos2(region.left(), overlap.bottom()), region.max),
                egui::Rect::from_min_max(
                    egui::pos2(region.left(), overlap.top()),
                    egui::pos2(overlap.left(), overlap.bottom()),
                ),
                egui::Rect::from_min_max(
                    egui::pos2(overlap.right(), overlap.top()),
                    egui::pos2(region.right(), overlap.bottom()),
                ),
            ] {
                if remainder.is_positive() {
                    next.push(remainder);
                }
            }
        }
        regions = next;
    }
    regions
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
    if window.is_clubgg_lobby {
        return match window.mode {
            WindowMode::Parked => "Lobby · parked bottom-right".to_owned(),
            WindowMode::TopRight => format!("Lobby · top-right · {}", status_text(window)),
            WindowMode::Ignored => "Lobby · ignored".to_owned(),
            WindowMode::FreeSpace => format!("Lobby · fills right side · {}", status_text(window)),
            WindowMode::Arranged => "Lobby · not a poker-table slot".to_owned(),
        };
    }
    match window.mode {
        WindowMode::Arranged => format!(
            "Table {} · {}",
            window.slot.unwrap_or_default(),
            window
                .status
                .as_ref()
                .map_or_else(|| "Ready".to_owned(), ToString::to_string)
        ),
        WindowMode::Parked => "Parked at top-right".to_owned(),
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
        SETTINGS_MAX_HEIGHT, SettingsViewportState, TableArrangerApp, application_overlay_rects,
        clickable_body_rects, settings_controls, slot_click_command, window_mode_rank,
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
    fn selected_table_clicking_another_number_emits_a_swap_command() {
        let mut selected = Some(WindowId(1));
        let destination = PokerSlotId::club(0, 1);

        let command = slot_click_command(&mut selected, Some(WindowId(2)), destination);
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
    fn table_body_hit_regions_do_not_cover_number_or_action_controls() {
        let tile = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(120.0, 80.0));
        let badge = egui::Rect::from_min_size(egui::pos2(3.0, 3.0), egui::vec2(16.0, 16.0));
        let actions = [
            egui::Rect::from_min_size(egui::pos2(34.0, 62.0), egui::vec2(16.0, 16.0)),
            egui::Rect::from_min_size(egui::pos2(52.0, 62.0), egui::vec2(16.0, 16.0)),
            egui::Rect::from_min_size(egui::pos2(70.0, 62.0), egui::vec2(16.0, 16.0)),
        ];
        let controls = [badge, actions[0], actions[1], actions[2]];
        let regions = clickable_body_rects(tile, &controls);

        for point in [
            egui::pos2(60.0, 30.0),
            egui::pos2(1.0, 79.0),
            egui::pos2(60.0, 79.0),
        ] {
            assert!(regions.iter().any(|region| region.contains(point)));
        }
        for control in controls {
            assert!(
                !regions
                    .iter()
                    .any(|region| region.contains(control.center()))
            );
        }
        let clickable_area: f32 = regions.iter().map(egui::Rect::area).sum();
        let control_area: f32 = controls.iter().map(egui::Rect::area).sum();
        assert!((clickable_area + control_area - tile.area()).abs() < 0.01);
    }

    #[test]
    fn application_overlay_scales_like_poker_columns() {
        let board = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(728.0, 394.0));
        let two = application_overlay_rects(board, 2);
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].left(), two[1].left());
        assert!(two[0].bottom() < two[1].bottom());
        assert!((two[0].height() - two[1].height()).abs() < 0.01);
        assert_eq!(two[0].left(), board.left() + 2.0);
        assert_eq!(two[0].right(), board.right() - 2.0);
        assert_eq!(two[1].bottom(), board.bottom() - 2.0);

        let four = application_overlay_rects(board, 4);
        assert_eq!(four.len(), 4);
        assert_eq!(four[0].left(), four[1].left());
        assert!(four[2].left() > four[0].left());
        assert_eq!(four[2].top(), four[0].top());
        assert!(four.iter().all(|rect| board.contains_rect(*rect)));
        assert_eq!(four[3].bottom(), board.bottom() - 2.0);
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
                    is_clubgg_lobby: index == 7,
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
            rng_overlay: crate::rng_overlay::RngOverlay::new(),
            selected_table: None,
            exiting: false,
        };

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
            ui.set_width(728.0);
            ui.set_height(116.0);
            let bounds = ui.max_rect();
            app.window_dock(ui);
            assert!(
                ui.min_rect().right() <= bounds.right() + 1.0,
                "window dock right {} exceeded bound {}",
                ui.min_rect().right(),
                bounds.right()
            );
            assert!(
                ui.min_rect().bottom() <= bounds.bottom() + 1.0,
                "window dock bottom {} exceeded bound {}",
                ui.min_rect().bottom(),
                bounds.bottom()
            );
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(728.0);
            ui.set_height(516.0);
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
            ui.set_height(super::desired_panel_height(&app.snapshot, 740.0));
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
        assert!((498.0..=499.0).contains(&super::desired_panel_height(&app.snapshot, 740.0)));
        assert!(super::desired_panel_height(&app.snapshot, 820.0) < super::PANEL_MAX_HEIGHT);
        assert_eq!(
            super::desired_panel_height(&UiSnapshot::default(), 740.0),
            180.0
        );
    }

    #[test]
    fn application_overlays_do_not_expand_panel_height() {
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
                    is_clubgg_lobby: false,
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

        assert_eq!(super::desired_panel_height(&snapshot, 740.0), 180.0);
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
            rng_overlay: crate::rng_overlay::RngOverlay::new(),
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
