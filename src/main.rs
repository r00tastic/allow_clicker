//! allow_clicker — GUI ekran OCR ile coklu hedef kelime tespit + otomatik tiklama

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clicker;
mod config;
mod hotkey;
mod ocr;
mod region_picker;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::{
    clicker::{Clicker, ClickerConfig, ClickerMsg},
    config::Config,
};

slint::include_modules!();

const MAX_LOG_LINES: usize = 100;

fn main() -> Result<()> {
    let ui = MainWindow::new()?;

    // --- Config yukle ---
    let cfg = Config::load();
    ui.set_case_sensitive(cfg.case_sensitive);
    ui.set_dry_run(cfg.dry_run);
    ui.set_region_x(cfg.region_x);
    ui.set_region_y(cfg.region_y);
    ui.set_region_w(cfg.region_w);
    ui.set_region_h(cfg.region_h);
    ui.set_interval_ms(cfg.interval_ms as f32);
    ui.set_cooldown_ms(cfg.cooldown_ms as f32);
    ui.set_status_text(SharedString::from("Bekleme"));

    // --- Hedef kelimeler modeli ---
    let targets_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(
        cfg.targets.iter().map(|s| SharedString::from(s.as_str())).collect::<Vec<_>>(),
    ));
    ui.set_targets(ModelRc::from(targets_model.clone()));

    // --- Clicker ---
    let clicker = Rc::new(RefCell::new(Clicker::new()));
    let (msg_tx, msg_rx) = mpsc::channel::<ClickerMsg>();

    let log_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // --- add-target ---
    {
        let model = targets_model.clone();
        ui.on_add_target(move |w| {
            let s = w.trim().to_string();
            if s.is_empty() { return; }
            // Dupe kontrolu (case-insensitive)
            let lower = s.to_lowercase();
            for i in 0..model.row_count() {
                if model.row_data(i).map(|r| r.to_lowercase() == lower).unwrap_or(false) {
                    return; // zaten var
                }
            }
            model.push(SharedString::from(s.as_str()));
        });
    }

    // --- remove-target ---
    {
        let model = targets_model.clone();
        ui.on_remove_target(move |idx| {
            let i = idx as usize;
            if i < model.row_count() && model.row_count() > 1 {
                model.remove(i);
            }
            // En az 1 kelime kalsin — hepsi silinmesin
        });
    }

    // --- Region picker ---
    {
        let ui_handle = ui.as_weak();
        ui.on_pick_region(move || {
            let (tx, rx) = mpsc::channel();
            region_picker::pick_region_async(tx);
            let ui_w = ui_handle.clone();
            std::thread::spawn(move || {
                if let Ok(opt) = rx.recv() {
                    if let Some(r) = opt {
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_w.upgrade() {
                                ui.set_region_x(r.x);
                                ui.set_region_y(r.y);
                                ui.set_region_w(r.w);
                                ui.set_region_h(r.h);
                            }
                        }).ok();
                    }
                }
            });
        });
    }

    // --- Toggle start/stop ---
    let toggle = {
        let ui_handle = ui.as_weak();
        let clicker = clicker.clone();
        let msg_tx = msg_tx.clone();
        let model = targets_model.clone();
        move || {
            let ui = match ui_handle.upgrade() { Some(u) => u, None => return };
            let mut c = clicker.borrow_mut();
            if c.is_running() {
                c.stop();
                ui.set_running(false);
                ui.set_status_text(SharedString::from("Durduruluyor..."));
            } else {
                // Model'den kelimeleri topla
                let mut targets: Vec<String> = Vec::with_capacity(model.row_count());
                for i in 0..model.row_count() {
                    if let Some(w) = model.row_data(i) {
                        let s = w.to_string();
                        if !s.trim().is_empty() {
                            targets.push(s);
                        }
                    }
                }
                if targets.is_empty() {
                    ui.set_status_text(SharedString::from("Hedef listesi bos!"));
                    return;
                }
                let cfg = ClickerConfig {
                    targets,
                    case_sensitive: ui.get_case_sensitive(),
                    dry_run: ui.get_dry_run(),
                    region_x: ui.get_region_x(),
                    region_y: ui.get_region_y(),
                    region_w: ui.get_region_w().max(0) as u32,
                    region_h: ui.get_region_h().max(0) as u32,
                    interval_ms: ui.get_interval_ms() as u64,
                    cooldown_ms: ui.get_cooldown_ms() as u64,
                };
                if let Err(e) = c.start(cfg, msg_tx.clone()) {
                    ui.set_status_text(SharedString::from(format!("HATA: {}", e)));
                    return;
                }
                ui.set_running(true);
                ui.set_status_text(SharedString::from("Tarama"));
            }
        }
    };

    {
        let toggle = toggle.clone();
        ui.on_toggle_running(move || toggle());
    }

    // --- Save config ---
    {
        let ui_handle = ui.as_weak();
        let model = targets_model.clone();
        ui.on_save_config(move || {
            let ui = match ui_handle.upgrade() { Some(u) => u, None => return };
            let mut targets: Vec<String> = Vec::with_capacity(model.row_count());
            for i in 0..model.row_count() {
                if let Some(w) = model.row_data(i) {
                    targets.push(w.to_string());
                }
            }
            let cfg = Config {
                targets,
                case_sensitive: ui.get_case_sensitive(),
                dry_run: ui.get_dry_run(),
                region_x: ui.get_region_x(),
                region_y: ui.get_region_y(),
                region_w: ui.get_region_w(),
                region_h: ui.get_region_h(),
                interval_ms: ui.get_interval_ms() as u32,
                cooldown_ms: ui.get_cooldown_ms() as u32,
            };
            match cfg.save() {
                Ok(_) => ui.set_status_text(SharedString::from("Ayarlar kaydedildi")),
                Err(e) => ui.set_status_text(SharedString::from(format!("Kayit hata: {}", e))),
            }
        });
    }

    // --- Clear log ---
    {
        let ui_handle = ui.as_weak();
        let log_lines = log_lines.clone();
        ui.on_clear_log(move || {
            log_lines.lock().unwrap().clear();
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_log_text(SharedString::from(""));
            }
        });
    }

    // --- F8 global hotkey ---
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();
    let _hk_guard = hotkey::install_f8(hotkey_tx).ok();

    // --- UI tick timer ---
    let ui_handle_tick = ui.as_weak();
    let log_lines_tick = log_lines.clone();
    let toggle_tick = toggle.clone();
    let start_time: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let start_time_tick = start_time.clone();
    let prev_uptime: Rc<Cell<u64>> = Rc::new(Cell::new(u64::MAX));
    let prev_uptime_tick = prev_uptime.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
        while hotkey_rx.try_recv().is_ok() {
            toggle_tick();
        }
        let mut log_dirty = false;
        let mut new_click = 0i32;
        let mut stopped = false;
        while let Ok(m) = msg_rx.try_recv() {
            match m {
                ClickerMsg::Log(line) => {
                    let mut lines = log_lines_tick.lock().unwrap();
                    lines.push(line);
                    if lines.len() > MAX_LOG_LINES {
                        let excess = lines.len() - MAX_LOG_LINES;
                        lines.drain(0..excess);
                    }
                    log_dirty = true;
                }
                ClickerMsg::Clicked => new_click += 1,
                ClickerMsg::Stopped => stopped = true,
            }
        }
        if let Some(ui) = ui_handle_tick.upgrade() {
            if log_dirty {
                let text = log_lines_tick.lock().unwrap().join("\n");
                ui.set_log_text(SharedString::from(text));
            }
            if new_click > 0 {
                let curr = ui.get_click_count();
                ui.set_click_count(curr + new_click);
            }
            if stopped {
                ui.set_running(false);
                ui.set_status_text(SharedString::from("Durduruldu"));
                start_time_tick.set(None);
            }
            let is_running = ui.get_running();
            if is_running && start_time_tick.get().is_none() {
                start_time_tick.set(Some(Instant::now()));
            }
            if !is_running && start_time_tick.get().is_some() {
                start_time_tick.set(None);
            }
            let secs = start_time_tick.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
            if secs != prev_uptime_tick.get() {
                prev_uptime_tick.set(secs);
                let up = format!("{:02}:{:02}", secs / 60, secs % 60);
                ui.set_uptime_text(SharedString::from(up));
            }
        }
    });

    ui.run()?;
    Ok(())
}
