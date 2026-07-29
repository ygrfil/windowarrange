use crossbeam_channel::Sender;
use eframe::egui;
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::identity::PRODUCT_NAME;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    ShowPanel,
    ArrangeNow,
    Exit,
}

pub struct TrayService {
    _icon: TrayIcon,
}

impl TrayService {
    pub fn new(context: egui::Context, sender: Sender<TrayAction>) -> Result<Self, String> {
        let menu = Menu::new();
        let show = MenuItem::with_id("show-panel", "Show control panel", true, None);
        let arrange = MenuItem::with_id("arrange-now", "Arrange now", true, None);
        let exit = MenuItem::with_id("exit", "Exit", true, None);
        menu.append_items(&[&show, &arrange, &exit])
            .map_err(|error| error.to_string())?;

        let menu_sender = sender.clone();
        let menu_context = context.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                "show-panel" => Some(TrayAction::ShowPanel),
                "arrange-now" => Some(TrayAction::ArrangeNow),
                "exit" => Some(TrayAction::Exit),
                _ => None,
            };
            if let Some(action) = action {
                let _ = menu_sender.try_send(action);
                menu_context.request_repaint();
            }
        }));

        let click_sender = sender;
        let click_context = context;
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = click_sender.try_send(TrayAction::ShowPanel);
                click_context.request_repaint();
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_tooltip(PRODUCT_NAME)
            .with_icon(make_icon()?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| error.to_string())?;

        Ok(Self { _icon: icon })
    }
}

fn make_icon() -> Result<tray_icon::Icon, String> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let border = x < 2 || y < 2 || x >= SIZE - 2 || y >= SIZE - 2;
            let grid = (x == 15 || x == 16 || y == 15 || y == 16)
                && (4..28).contains(&x)
                && (4..28).contains(&y);
            let table = (4..28).contains(&x) && (4..28).contains(&y);
            let color = if border {
                [17, 24, 39, 255]
            } else if grid {
                [244, 248, 255, 255]
            } else if table {
                [22, 163, 74, 255]
            } else {
                [34, 197, 94, 255]
            };
            rgba.extend_from_slice(&color);
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).map_err(|error| error.to_string())
}

#[must_use]
pub fn egui_icon() -> egui::IconData {
    const SIZE: u32 = 32;
    let icon = make_rgba();
    egui::IconData {
        rgba: icon,
        width: SIZE,
        height: SIZE,
    }
}

fn make_rgba() -> Vec<u8> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let border = x < 2 || y < 2 || x >= SIZE - 2 || y >= SIZE - 2;
            let grid = (x == 15 || x == 16 || y == 15 || y == 16)
                && (4..28).contains(&x)
                && (4..28).contains(&y);
            let table = (4..28).contains(&x) && (4..28).contains(&y);
            let color = if border {
                [17, 24, 39, 255]
            } else if grid {
                [244, 248, 255, 255]
            } else if table {
                [22, 163, 74, 255]
            } else {
                [34, 197, 94, 255]
            };
            rgba.extend_from_slice(&color);
        }
    }
    rgba
}
