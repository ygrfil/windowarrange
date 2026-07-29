use crossbeam_channel::Sender;
use eframe::egui;
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

#[path = "../assets/icon.rs"]
mod app_icon;

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
    tray_icon::Icon::from_rgba(make_rgba(), app_icon::SIZE, app_icon::SIZE)
        .map_err(|error| error.to_string())
}

#[must_use]
pub fn egui_icon() -> egui::IconData {
    let icon = make_rgba();
    egui::IconData {
        rgba: icon,
        width: app_icon::SIZE,
        height: app_icon::SIZE,
    }
}

fn make_rgba() -> Vec<u8> {
    let mut rgba = Vec::with_capacity((app_icon::SIZE * app_icon::SIZE * 4) as usize);
    for y in 0..app_icon::SIZE {
        for x in 0..app_icon::SIZE {
            rgba.extend_from_slice(&app_icon::rgba_pixel(x, y));
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::app_icon;

    #[test]
    fn icon_is_blue_with_a_spade_and_centered_white_plus() {
        assert_eq!(app_icon::rgba_pixel(3, 3), [37, 99, 235, 255]);
        assert_eq!(app_icon::rgba_pixel(15, 5), [15, 45, 95, 255]);
        assert_eq!(app_icon::rgba_pixel(15, 15), [255, 255, 255, 255]);
        assert_eq!(app_icon::rgba_pixel(10, 15), [255, 255, 255, 255]);
    }
}
