use std::{collections::HashMap, str::FromStr};

use crossbeam_channel::Sender;
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};

use crate::config::HotkeySettings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeyAction {
    ArrangeNow,
    ToggleFocused,
    TogglePanel,
    ToggleSlot(usize),
}

pub struct HotkeyService {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: HashMap<u32, HotkeyAction>,
}

impl HotkeyService {
    pub fn new(
        settings: &HotkeySettings,
        context: egui::Context,
        event_sender: Sender<u32>,
    ) -> Result<(Self, Vec<String>), String> {
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = event_sender.try_send(event.id);
                context.request_repaint();
            }
        }));

        let manager = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        let mut service = Self {
            manager,
            registered: Vec::new(),
            actions: HashMap::new(),
        };
        let errors = service.apply(settings);
        Ok((service, errors))
    }

    pub fn apply(&mut self, settings: &HotkeySettings) -> Vec<String> {
        if !self.registered.is_empty() {
            let _ = self.manager.unregister_all(&self.registered);
        }
        self.registered.clear();
        self.actions.clear();

        let mut requested = vec![
            (&settings.arrange_now, HotkeyAction::ArrangeNow),
            (&settings.toggle_focused, HotkeyAction::ToggleFocused),
            (&settings.show_panel, HotkeyAction::TogglePanel),
        ];
        requested.extend(
            settings
                .toggle_slots
                .iter()
                .take(8)
                .enumerate()
                .map(|(slot, shortcut)| (shortcut, HotkeyAction::ToggleSlot(slot))),
        );

        let mut errors = Vec::new();
        for (shortcut, action) in requested {
            let canonical = canonicalize(shortcut);
            let hotkey = match HotKey::from_str(&canonical) {
                Ok(hotkey) => hotkey,
                Err(error) => {
                    errors.push(format!("{shortcut}: {error}"));
                    continue;
                }
            };
            match self.manager.register(hotkey) {
                Ok(()) => {
                    self.actions.insert(hotkey.id(), action);
                    self.registered.push(hotkey);
                }
                Err(error) => errors.push(format!("{shortcut}: {error}")),
            }
        }
        errors
    }

    #[must_use]
    pub fn action_for(&self, id: u32) -> Option<HotkeyAction> {
        self.actions.get(&id).copied()
    }
}

fn canonicalize(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" => "control".to_owned(),
            other => other.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.manager.unregister_all(&self.registered);
    }
}

#[cfg(test)]
mod tests {
    use super::canonicalize;

    #[test]
    fn ctrl_alias_is_canonicalized() {
        assert_eq!(canonicalize("Ctrl+Shift+F1"), "control+shift+f1");
    }
}
