//! Keyboard shortcut matching.
//!
//! Accelerators come from `[keybindings]` as GTK accelerator strings. Matching
//! is robust to Shift changing the keyval case (Ctrl+Shift+X is delivered as
//! keyval `X`): both sides are lower-cased and only Ctrl/Shift/Alt/Super are
//! compared.

use gtk::gdk::{Key, ModifierType};

use crate::config::{parse_accel, Keybindings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Launch,
    Close,
    Next,
    Previous,
    HideSelected,
    HiddenManager,
    ClearSearch,
    ContextMenu,
}

fn mask() -> ModifierType {
    ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK
}

#[derive(Debug, Clone, Default)]
pub struct KeyMap {
    bindings: Vec<(Action, Key, ModifierType)>,
}

impl KeyMap {
    pub fn from_config(k: &Keybindings) -> Self {
        let mut bindings = Vec::new();
        let groups: [(Action, &Vec<String>); 8] = [
            (Action::Launch, &k.launch),
            (Action::Close, &k.close),
            (Action::Next, &k.next),
            (Action::Previous, &k.previous),
            (Action::HideSelected, &k.hide_selected),
            (Action::HiddenManager, &k.hidden_manager),
            (Action::ClearSearch, &k.clear_search),
            (Action::ContextMenu, &k.context_menu),
        ];
        for (action, accels) in groups {
            for accel in accels {
                if let Some((key, mods)) = parse_accel(accel) {
                    bindings.push((action, key.to_lower(), mods & mask()));
                }
            }
        }
        Self { bindings }
    }

    /// Finds the action bound to a key press, if any.
    pub fn lookup(&self, key: Key, state: ModifierType) -> Option<Action> {
        let key = key.to_lower();
        let state = state & mask();
        self.bindings.iter().find(|(_, k, m)| *k == key && *m == state).map(|(a, _, _)| *a)
    }

    /// Human-readable label of the first shortcut of `action` ("Ctrl+Shift+H").
    pub fn label(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .find(|(a, _, _)| *a == action)
            .map(|(_, k, m)| gtk::accelerator_get_label(*k, *m).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_changes_keyval_case_but_still_matches() {
        let map = KeyMap::from_config(&Keybindings::default());
        let ctrl_shift = ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK;
        assert_eq!(map.lookup(Key::X, ctrl_shift), Some(Action::HideSelected));
        assert_eq!(map.lookup(Key::x, ctrl_shift), Some(Action::HideSelected));
        assert_eq!(map.lookup(Key::H, ctrl_shift | ModifierType::LOCK_MASK), Some(Action::HiddenManager));
        assert_eq!(map.lookup(Key::x, ModifierType::CONTROL_MASK), None);
        assert_eq!(map.lookup(Key::Down, ModifierType::empty()), Some(Action::Next));
        assert_eq!(map.lookup(Key::n, ModifierType::CONTROL_MASK), Some(Action::Next));
        assert_eq!(map.lookup(Key::Down, ModifierType::CONTROL_MASK), None);
        assert_eq!(map.lookup(Key::Return, ModifierType::empty()), Some(Action::Launch));
        assert_eq!(map.lookup(Key::Escape, ModifierType::empty()), Some(Action::Close));
        assert_eq!(map.lookup(Key::F10, ModifierType::SHIFT_MASK), Some(Action::ContextMenu));
    }
}
