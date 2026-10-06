//! F8 global kisayol — pencere arkada iken bile start/stop.

use std::sync::mpsc;

use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

pub struct HotkeyGuard {
    // manager drop olunca kisayol devredisi kalir
    _manager: GlobalHotKeyManager,
}

pub fn install_f8(tx: mpsc::Sender<()>) -> Result<HotkeyGuard> {
    let manager = GlobalHotKeyManager::new()?;
    let hk = HotKey::new(Some(Modifiers::empty()), Code::F8);
    manager.register(hk)?;

    // Alici thread: event geldiginde tx'e sinyal
    std::thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(ev) = receiver.recv() {
            if ev.state == HotKeyState::Pressed {
                let _ = tx.send(());
            }
        }
    });

    Ok(HotkeyGuard { _manager: manager })
}
