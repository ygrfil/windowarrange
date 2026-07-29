use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, bounded};
use eframe::egui;
use tracing::{error, info, warn};

use crate::{
    config::{AppConfig, ConfigStore, HotkeySettings},
    controller::{ControllerCommand, ControllerHandle, spawn_controller_with_waker},
    hotkeys::{HotkeyAction, HotkeyService},
    identity::{APP_ID, PANEL_TITLE},
    logging,
    model::{CandidateView, TableStatus, UiSnapshot, WindowMode},
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

pub fn run() {
    let mitigation_result = apply_process_mitigations();
    let store = ConfigStore::for_current_user();
    let log_path = store.log_path();
    let logging_result = logging::initialize(&log_path);
    std::panic::set_hook(Box::new(|panic| {
        error!(%panic, "application panic");
    }));
    match logging_result {
        Ok(path) => info!(
            version = env!("CARGO_PKG_VERSION"),
            log_file = %path.display(),
            "Table Arranger Control started"
        ),
        Err(error) => {
            eprintln!("Could not initialize file logging: {error}");
        }
    }
    match mitigation_result {
        Ok(()) => info!("legacy extension-point DLL loading is disabled"),
        Err(error) => warn!(%error, "could not enable process extension-point mitigation"),
    }

    let _instance = match acquire_single_instance() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            activate_existing_panel();
            return;
        }
        Err(error) => {
            error!(%error, "single-instance setup failed");
            return;
        }
    };

    let config = match store.load() {
        Ok(config) => config,
        Err(error) => {
            warn!(%error, "using default configuration");
            AppConfig::default()
        }
    };
    let hotkey_settings = config.hotkeys.clone();
    let backend = Win32Backend::new();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(PANEL_TITLE)
            .with_inner_size([740.0, 380.0])
            .with_min_inner_size([680.0, 350.0])
            .with_max_inner_size([820.0, 460.0])
            .with_maximize_button(false)
            .with_always_on_top()
            .with_icon(Arc::new(egui_icon())),
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
            )))
        }),
    );
    if let Err(error) = result {
        error!(%error, "application exited with an error");
    }
}

struct TableArrangerApp {
    commands: Sender<ControllerCommand>,
    snapshots: Receiver<UiSnapshot>,
    snapshot: UiSnapshot,
    hotkeys: Option<HotkeyService>,
    hotkey_events: Receiver<u32>,
    _tray: Option<TrayService>,
    tray_events: Receiver<TrayAction>,
    shortcut_draft: HotkeySettings,
    shortcut_errors: Vec<String>,
    settings_open: bool,
    dragged_table: Option<crate::model::WindowId>,
    exiting: bool,
}

impl TableArrangerApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        controller: ControllerHandle,
        hotkey_settings: HotkeySettings,
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
                warn!(%error, "tray icon unavailable");
                None
            }
        };

        Self {
            commands: controller.commands,
            snapshots: controller.snapshots,
            snapshot: UiSnapshot::default(),
            hotkeys,
            hotkey_events: hotkey_rx,
            _tray: tray,
            tray_events: tray_rx,
            shortcut_draft: hotkey_settings,
            shortcut_errors,
            settings_open: false,
            dragged_table: None,
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
            self.snapshot = snapshot;
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
        let arranged = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.mode == WindowMode::Arranged)
            .count();
        let parked = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| window.mode == WindowMode::Parked)
            .count();
        let positioned = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| matches!(window.mode, WindowMode::TopRight | WindowMode::FreeSpace))
            .count();
        let ignored = self
            .snapshot
            .candidates
            .len()
            .saturating_sub(arranged + parked + positioned);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Workspace");
                ui.label(
                    egui::RichText::new(format!(
                        "{arranged} tables  ·  {positioned} positioned  ·  {parked} parked  ·  {ignored} ignored"
                    ))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized([26.0, 24.0], egui::Button::new("-"))
                    .on_hover_text("Hide to tray")
                    .clicked()
                {
                    Self::hide_panel(ui.ctx());
                }

                let selected_text = self
                    .snapshot
                    .monitors
                    .iter()
                    .find(|monitor| {
                        Some(&monitor.id) == self.snapshot.selected_monitor.as_ref()
                    })
                    .map_or("Select display", |monitor| monitor.label.as_str());
                egui::ComboBox::from_id_salt("target-display")
                    .selected_text(selected_text)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for monitor in &self.snapshot.monitors {
                            let selected =
                                Some(&monitor.id) == self.snapshot.selected_monitor.as_ref();
                            if ui.selectable_label(selected, &monitor.label).clicked() {
                                self.send(ControllerCommand::SelectMonitor(monitor.id.clone()));
                            }
                        }
                    });
                ui.label(
                    egui::RichText::new("Display")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        })
        .response
    }

    fn control_rail(&mut self, ui: &mut egui::Ui) {
        let rail_height = ui.available_height();
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(7)
            .inner_margin(egui::Margin::symmetric(4, 5))
            .show(ui, |ui| {
                ui.set_width(52.0);
                ui.set_min_height((rail_height - 10.0).max(0.0));
                let mut automatic = self.snapshot.auto_arrange;
                if ui
                    .add_sized(
                        [52.0, 26.0],
                        egui::Button::new(if automatic { "Auto ON" } else { "Auto OFF" })
                            .selected(automatic),
                    )
                    .on_hover_text("Automatically reapply the complete workspace after changes")
                    .clicked()
                {
                    automatic = !automatic;
                    self.send(ControllerCommand::SetAutoArrange(automatic));
                }
                if ui
                    .add_sized(
                        [52.0, 26.0],
                        egui::Button::new(
                            egui::RichText::new("Arrange").color(egui::Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .corner_radius(7),
                    )
                    .on_hover_text("Arrange poker tables and selected application windows now")
                    .clicked()
                {
                    self.send(ControllerCommand::ForceArrange);
                }
                if ui
                    .add_sized([52.0, 26.0], egui::Button::new("Refresh"))
                    .on_hover_text("Discover windows again and reapply selected placement")
                    .clicked()
                {
                    self.send(ControllerCommand::Refresh);
                }
                if ui
                    .add_sized([52.0, 26.0], egui::Button::new("Settings"))
                    .on_hover_text("Hotkeys and settings")
                    .clicked()
                {
                    self.settings_open = true;
                }
            });
    }

    fn workspace(&mut self, ui: &mut egui::Ui) {
        ui.add_space(5.0);
        if self.snapshot.candidates.is_empty() {
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(10)
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No application windows found").strong());
                        ui.label(
                            egui::RichText::new("Open an application, then press Refresh.")
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
            .filter(|window| window.is_clubgg)
            .cloned()
            .collect();

        if windows.is_empty() {
            empty_section(ui, "No ClubGG windows", "Open a table and press Refresh.");
            return;
        }

        let mut active: Vec<_> = windows
            .iter()
            .filter(|window| window.mode == WindowMode::Arranged)
            .cloned()
            .collect();
        let mut inactive: Vec<_> = windows
            .iter()
            .filter(|window| window.mode != WindowMode::Arranged)
            .cloned()
            .collect();
        active.sort_by_key(|window| window.slot);
        inactive.sort_by_key(|window| window_mode_rank(window.mode));

        let content_width = ui.available_width();
        let tile_width = ((content_width - 4.0) / 2.0).max(90.0);
        let active_rows = active.len().div_ceil(2);
        let inactive_rows = inactive.len().div_ceil(2);
        let tile_height =
            grouped_poker_tile_height(ui.available_height(), active_rows, inactive_rows);

        self.poker_group(ui, &active, tile_width, tile_height);
        if !active.is_empty() && !inactive.is_empty() {
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(2.0);
        }
        self.poker_group(ui, &inactive, tile_width, tile_height);

        if ui.input(|input| input.pointer.any_released()) {
            self.dragged_table = None;
        }
    }

    fn poker_group(
        &mut self,
        ui: &mut egui::Ui,
        windows: &[CandidateView],
        tile_width: f32,
        tile_height: f32,
    ) {
        if windows.is_empty() {
            return;
        }

        let row_count = windows.len().div_ceil(2);
        for (row_index, row) in windows.chunks(2).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for window in row {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, tile_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.poker_tile(ui, window),
                    );
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(4.0);
            }
        }
    }

    fn poker_tile(&mut self, ui: &mut egui::Ui, window: &CandidateView) {
        let is_dragging = self.dragged_table == Some(window.id);
        let card_fill = if is_dragging {
            ACCENT.gamma_multiply(0.22)
        } else if ui.visuals().dark_mode {
            egui::Color32::from_gray(27)
        } else {
            egui::Color32::from_gray(248)
        };
        let card_stroke = if is_dragging {
            egui::Stroke::new(2.0, ACCENT)
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        let mut drag_started = false;
        let response = egui::Frame::new()
            .fill(card_fill)
            .stroke(card_stroke)
            .corner_radius(6)
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    let drag = ui
                        .add_enabled(
                            window.slot.is_some(),
                            egui::Label::new("::").sense(egui::Sense::drag()),
                        )
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text("Drag onto another table to swap positions");
                    drag_started = drag.drag_started();
                    compact_slot_badge(ui, window);
                    let title_width = (ui.available_width() - 26.0).max(24.0);
                    ui.add_sized(
                        [title_width, 16.0],
                        egui::Label::new(egui::RichText::new(&window.label).strong()).truncate(),
                    )
                    .on_hover_text(&window.label);
                    if ui
                        .add_sized(
                            [18.0, 18.0],
                            egui::Button::new(egui::RichText::new("◎").size(9.0)),
                        )
                        .on_hover_text("Locate this window")
                        .clicked()
                    {
                        self.send(ControllerCommand::Highlight(window.id));
                    }
                });
                self.poker_window_controls(ui, window);
            })
            .response
            .on_hover_text(window_subtitle(window));

        if drag_started {
            self.dragged_table = Some(window.id);
        }
        if let Some(dragged) = self.dragged_table
            && dragged != window.id
            && response.hovered()
            && ui.input(|input| input.pointer.any_released())
        {
            let from = self
                .snapshot
                .tables
                .iter()
                .position(|table| table.id == dragged);
            let to = self
                .snapshot
                .tables
                .iter()
                .position(|table| table.id == window.id);
            if let (Some(from), Some(to)) = (from, to) {
                self.send(ControllerCommand::Reorder { from, to });
            }
            self.dragged_table = None;
        }
    }

    fn application_board(&mut self, ui: &mut egui::Ui) {
        let mut windows: Vec<_> = self
            .snapshot
            .candidates
            .iter()
            .filter(|window| !window.is_clubgg)
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
        let rows = windows.len().div_ceil(2);
        let tile_height = dense_tile_height(ui.available_height(), rows);
        for row in windows.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for window in row {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, tile_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.application_tile(ui, window),
                    );
                }
            });
            ui.add_space(4.0);
        }
    }

    fn status_line(&self, ui: &mut egui::Ui) {
        let has_failure = self.snapshot.tables.iter().any(|table| {
            matches!(
                table.status,
                TableStatus::AccessDenied | TableStatus::MoveFailed(_)
            )
        }) || self.snapshot.candidates.iter().any(|window| {
            matches!(
                window.status,
                Some(TableStatus::AccessDenied | TableStatus::MoveFailed(_))
            )
        });
        let color = if has_failure {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().weak_text_color()
        };
        ui.label(
            egui::RichText::new(&self.snapshot.status_message)
                .small()
                .color(color),
        );
    }

    fn application_tile(&mut self, ui: &mut egui::Ui, window: &CandidateView) {
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
                    compact_slot_badge(ui, window);
                    let text_width = (ui.available_width() - 24.0).max(24.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 28.0),
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
                    if ui
                        .small_button("◎")
                        .on_hover_text("Locate this window")
                        .clicked()
                    {
                        self.send(ControllerCommand::Highlight(window.id));
                    }
                });
                ui.add_space(1.0);
                self.application_window_controls(ui, window);
            });
    }

    fn poker_window_controls(&self, ui: &mut egui::Ui, window: &CandidateView) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = ((ui.available_width() - 4.0) / 3.0).max(20.0);
            if mode_button(
                ui,
                width,
                "Active",
                window.mode == WindowMode::Arranged,
                ACCENT,
            )
            .on_hover_text("Include in the equal-size poker-table grid")
            .clicked()
            {
                self.set_window_mode(window, WindowMode::Arranged);
            }
            if mode_button(ui, width, "Park", window.mode == WindowMode::Parked, PARKED)
                .on_hover_text("Shrink and overlap at the bottom-right")
                .clicked()
            {
                self.set_window_mode(window, WindowMode::Parked);
            }
            if mode_button(
                ui,
                width,
                "Ignore",
                window.mode == WindowMode::Ignored,
                egui::Color32::from_gray(100),
            )
            .on_hover_text("Do not move this window")
            .clicked()
            {
                self.set_window_mode(window, WindowMode::Ignored);
            }
        });
    }

    fn application_window_controls(&self, ui: &mut egui::Ui, window: &CandidateView) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = ((ui.available_width() - 4.0) / 3.0).max(20.0);
            if mode_button(
                ui,
                width,
                "Ignore",
                window.mode == WindowMode::Ignored,
                egui::Color32::from_gray(100),
            )
            .on_hover_text("Leave this application window untouched")
            .clicked()
            {
                self.set_window_mode(window, WindowMode::Ignored);
            }
            if mode_button(
                ui,
                width,
                "Free",
                window.mode == WindowMode::FreeSpace,
                FREE_SPACE,
            )
            .on_hover_text("Fill only the vertical space to the right of active poker tables")
            .clicked()
            {
                self.set_window_mode(window, WindowMode::FreeSpace);
            }
            if mode_button(
                ui,
                width,
                "Top",
                window.mode == WindowMode::TopRight,
                TOP_RIGHT,
            )
            .on_hover_text("Keep its size and anchor it to the display's top-right")
            .clicked()
            {
                self.set_window_mode(window, WindowMode::TopRight);
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

    fn settings_window(&mut self, context: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Hotkeys")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(context, |ui| {
                ui.label(
                    egui::RichText::new("Global shortcuts")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                shortcut_field(ui, "Arrange", &mut self.shortcut_draft.arrange_now);
                shortcut_field(
                    ui,
                    "Toggle focused",
                    &mut self.shortcut_draft.toggle_focused,
                );
                shortcut_field(ui, "Show panel", &mut self.shortcut_draft.show_panel);
                ui.separator();
                for (index, shortcut) in self.shortcut_draft.toggle_slots.iter_mut().enumerate() {
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
                    self.shortcut_errors = self.hotkeys.as_mut().map_or_else(
                        || vec!["Global hotkey service is unavailable.".to_owned()],
                        |service| service.apply(&self.shortcut_draft),
                    );
                    self.send(ControllerCommand::SetHotkeys(self.shortcut_draft.clone()));
                }
                for error in &self.shortcut_errors {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Version {} · administrator mode",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
        self.settings_open = open;
    }

    fn workspace_row(&mut self, ui: &mut egui::Ui) -> [egui::Rect; 2] {
        let mut rail_rect = egui::Rect::NOTHING;
        let mut workspace_rect = egui::Rect::NOTHING;
        ui.horizontal_top(|ui| {
            let height = ui.available_height();
            rail_rect = ui
                .allocate_ui_with_layout(
                    egui::vec2(60.0, height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| self.control_rail(ui),
                )
                .response
                .rect;
            workspace_rect = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.workspace(ui),
                )
                .response
                .rect;
        });
        [rail_rect, workspace_rect]
    }
}

impl eframe::App for TableArrangerApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_background_events(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style())
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.take_available_space();
                self.top_bar(ui);
                self.status_line(ui);
                ui.separator();
                let _ = self.workspace_row(ui);
            });
        if self.settings_open {
            self.settings_window(ui.ctx());
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.send(ControllerCommand::Shutdown);
        info!("Table Arranger Control stopped");
    }
}

fn compact_slot_badge(ui: &mut egui::Ui, window: &CandidateView) {
    let color = match window.mode {
        WindowMode::Arranged => ACCENT,
        WindowMode::Parked => PARKED,
        WindowMode::TopRight => TOP_RIGHT,
        WindowMode::FreeSpace => FREE_SPACE,
        WindowMode::Ignored => ui.visuals().widgets.inactive.bg_fill,
    };
    let text = match window.mode {
        WindowMode::TopRight => "↗".to_owned(),
        WindowMode::FreeSpace => "□".to_owned(),
        _ => window
            .slot
            .map_or_else(|| "—".to_owned(), |slot| slot.to_string()),
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(9.0),
        if window.mode == WindowMode::Ignored {
            ui.visuals().text_color()
        } else {
            egui::Color32::WHITE
        },
    );
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

fn mode_button(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    selected: bool,
    color: egui::Color32,
) -> egui::Response {
    let text = if selected {
        egui::RichText::new(label).color(egui::Color32::WHITE)
    } else {
        egui::RichText::new(label)
    };
    let mut button = egui::Button::new(text).corner_radius(4).selected(selected);
    if selected {
        button = button.fill(color);
    }
    ui.add_sized([width, 18.0], button)
}

fn dense_tile_height(available_height: f32, rows: usize) -> f32 {
    if rows == 0 {
        return 0.0;
    }
    let gaps = (rows.saturating_sub(1) as f32) * 4.0;
    ((available_height - gaps) / rows as f32).clamp(48.0, 62.0)
}

fn grouped_poker_tile_height(
    available_height: f32,
    active_rows: usize,
    inactive_rows: usize,
) -> f32 {
    let rows = active_rows + inactive_rows;
    if rows == 0 {
        return 0.0;
    }
    let row_gaps = (active_rows.saturating_sub(1) + inactive_rows.saturating_sub(1)) as f32 * 4.0;
    let group_chrome = if active_rows > 0 && inactive_rows > 0 {
        8.0
    } else {
        0.0
    };
    ((available_height - row_gaps - group_chrome) / rows as f32).clamp(44.0, 62.0)
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
    use crossbeam_channel::unbounded;
    use eframe::egui;

    use super::{TableArrangerApp, window_mode_rank};
    use crate::{
        config::HotkeySettings,
        controller::ControllerCommand,
        model::{CandidateView, UiSnapshot, WindowId, WindowMode},
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
    fn workspace_content_stays_inside_a_compact_panel() {
        let (commands, _command_rx) = unbounded::<ControllerCommand>();
        let (_snapshot_tx, snapshots) = unbounded::<UiSnapshot>();
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
                    is_clubgg: index < 8,
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
            ..UiSnapshot::default()
        };
        let mut app = TableArrangerApp {
            commands,
            snapshots,
            snapshot,
            hotkeys: None,
            hotkey_events,
            _tray: None,
            tray_events,
            shortcut_draft: HotkeySettings::default(),
            shortcut_errors: Vec::new(),
            settings_open: false,
            dragged_table: None,
            exiting: false,
        };

        let first_table = app.snapshot.candidates[0].clone();
        egui::__run_test_ui(|ui| {
            ui.set_width(166.0);
            ui.set_height(62.0);
            let bounds = ui.max_rect();
            app.poker_tile(ui, &first_table);
            assert!(
                ui.min_rect().right() <= bounds.right() + 1.0,
                "poker tile right {} exceeded bound {}",
                ui.min_rect().right(),
                bounds.right()
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
            ui.set_width(700.0);
            ui.set_height(280.0);
            let [rail, workspace] = app.workspace_row(ui);
            assert!(
                (rail.top() - workspace.top()).abs() <= 1.0,
                "rail top {} did not align with workspace top {}",
                rail.top(),
                workspace.top()
            );
        });
    }

    #[test]
    fn top_bar_never_consumes_the_workspace_height() {
        let (commands, _command_rx) = unbounded::<ControllerCommand>();
        let (_snapshot_tx, snapshots) = unbounded::<UiSnapshot>();
        let (_hotkey_tx, hotkey_events) = unbounded::<u32>();
        let (_tray_tx, tray_events) = unbounded::<TrayAction>();
        let mut app = TableArrangerApp {
            commands,
            snapshots,
            snapshot: UiSnapshot::default(),
            hotkeys: None,
            hotkey_events,
            _tray: None,
            tray_events,
            shortcut_draft: HotkeySettings::default(),
            shortcut_errors: Vec::new(),
            settings_open: false,
            dragged_table: None,
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
