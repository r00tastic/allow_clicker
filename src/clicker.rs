use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::Result;
use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
use image::DynamicImage;
use xcap::Monitor;

use crate::ocr::{Hit, Ocr};

#[derive(Debug, Clone)]
pub struct ClickerConfig {
    pub targets: Vec<String>,
    pub case_sensitive: bool,
    pub dry_run: bool,
    pub region_x: i32,
    pub region_y: i32,
    pub region_w: u32,
    pub region_h: u32,
    pub interval_ms: u64,
    pub cooldown_ms: u64,
}

#[derive(Debug, Clone)]
pub enum ClickerMsg {
    Log(String),
    Clicked,
    Stopped,
}

pub struct Clicker {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Clicker {
    pub fn new() -> Self {
        Self { running: Arc::new(AtomicBool::new(false)), handle: None }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn start(&mut self, cfg: ClickerConfig, tx: mpsc::Sender<ClickerMsg>) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        // Onceki worker'in join'ini garanti et (start restart senaryosu)
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.running.store(true, Ordering::Release);
        let running = self.running.clone();
        let handle = thread::spawn(move || {
            let _ = worker(cfg, running.clone(), tx.clone());
            running.store(false, Ordering::Release);
            let _ = tx.send(ClickerMsg::Stopped);
        });
        self.handle = Some(handle);
        Ok(())
    }

    /// Sadece sinyal ver — UI thread'i bloklamaz. Worker sonraki tick'te durur.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

impl Drop for Clicker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn send_log(tx: &mpsc::Sender<ClickerMsg>, msg: impl Into<String>) {
    let _ = tx.send(ClickerMsg::Log(format!("{} {}", ts(), msg.into())));
}

/// Region merkezini iceren monitoru bul; hicbiri kapsamıyorsa primary.
fn find_monitor_for_region(cfg: &ClickerConfig, monitors: &[Monitor]) -> Option<usize> {
    if cfg.region_w == 0 || cfg.region_h == 0 {
        return Some(0);
    }
    let cx = cfg.region_x + (cfg.region_w as i32) / 2;
    let cy = cfg.region_y + (cfg.region_h as i32) / 2;
    for (i, m) in monitors.iter().enumerate() {
        let mx = m.x();
        let my = m.y();
        let mw = m.width() as i32;
        let mh = m.height() as i32;
        if cx >= mx && cx < mx + mw && cy >= my && cy < my + mh {
            return Some(i);
        }
    }
    Some(0)
}

fn worker(
    cfg: ClickerConfig,
    running: Arc<AtomicBool>,
    tx: mpsc::Sender<ClickerMsg>,
) -> Result<()> {
    let ocr = match Ocr::new() {
        Ok(o) => o,
        Err(e) => {
            send_log(&tx, format!("[HATA] OCR: {}", e));
            return Err(e);
        }
    };

    let monitors = match Monitor::all() {
        Ok(m) if !m.is_empty() => m,
        _ => {
            send_log(&tx, "[HATA] Monitor bulunamadi");
            return Ok(());
        }
    };
    let mon_idx = find_monitor_for_region(&cfg, &monitors).unwrap_or(0);
    let monitor = &monitors[mon_idx];
    let mon_x = monitor.x();
    let mon_y = monitor.y();

    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            send_log(&tx, format!("[HATA] Enigo: {}", e));
            return Ok(());
        }
    };

    send_log(&tx, format!(
        "[+] baslatildi — hedefler={:?}, monitor#{} @({},{}), bolge=({},{},{},{}){}",
        cfg.targets, mon_idx, mon_x, mon_y,
        cfg.region_x, cfg.region_y, cfg.region_w, cfg.region_h,
        if cfg.dry_run { " [DRY-RUN]" } else { "" }
    ));

    while running.load(Ordering::Acquire) {
        let full = match monitor.capture_image() {
            Ok(im) => im,
            Err(e) => {
                send_log(&tx, format!("[!] capture: {}", e));
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        };
        let img_dyn = DynamicImage::ImageRgba8(full);
        let img_w = img_dyn.width();
        let img_h = img_dyn.height();

        // Bolgeyi monitor-relative koordinata cevir
        let (local_x, local_y, rw, rh) = if cfg.region_w == 0 || cfg.region_h == 0 {
            (0i32, 0i32, img_w, img_h)
        } else {
            let lx = cfg.region_x - mon_x;
            let ly = cfg.region_y - mon_y;
            // Ekran disina taşan kismi kirp
            let lx_c = lx.max(0);
            let ly_c = ly.max(0);
            let dx = (lx_c - lx) as u32; // baslangicin kırpildigi kadar geniisligi de kirp
            let dy = (ly_c - ly) as u32;
            let mut w = cfg.region_w.saturating_sub(dx);
            let mut h = cfg.region_h.saturating_sub(dy);
            if lx_c as u32 >= img_w || ly_c as u32 >= img_h {
                (0, 0, 0, 0)
            } else {
                w = w.min(img_w - lx_c as u32);
                h = h.min(img_h - ly_c as u32);
                (lx_c, ly_c, w, h)
            }
        };
        if rw == 0 || rh == 0 {
            send_log(&tx, "[!] bolge ekran disi");
            thread::sleep(Duration::from_millis(1000));
            continue;
        }
        let cropped = img_dyn.crop_imm(local_x as u32, local_y as u32, rw, rh);

        // OCR (goruntu koordinatinda tespit -> ekran mutlak koordinatina cevir)
        let hit: Option<Hit> = match ocr.find_any(
            &cropped,
            &cfg.targets,
            cfg.case_sensitive,
            mon_x + local_x,
            mon_y + local_y,
        ) {
            Ok(h) => h,
            Err(e) => {
                send_log(&tx, format!("[!] OCR: {}", e));
                thread::sleep(Duration::from_millis(cfg.interval_ms));
                continue;
            }
        };

        if let Some(h) = hit {
            send_log(&tx, format!("[+] '{}' bulundu @ ({}, {})", h.text, h.center_x, h.center_y));
            if !cfg.dry_run {
                if let Err(e) = enigo.move_mouse(h.center_x, h.center_y, Coordinate::Abs) {
                    send_log(&tx, format!("[!] mouse move: {}", e));
                    thread::sleep(Duration::from_millis(cfg.interval_ms));
                    continue;
                }
                thread::sleep(Duration::from_millis(60));
                if let Err(e) = enigo.button(Button::Left, Direction::Click) {
                    send_log(&tx, format!("[!] click: {}", e));
                }
                let _ = tx.send(ClickerMsg::Clicked);
                send_log(&tx, "    [OK] tiklandi");
                sleep_interruptible(&running, cfg.cooldown_ms);
            } else {
                sleep_interruptible(&running, cfg.interval_ms);
            }
        } else {
            sleep_interruptible(&running, cfg.interval_ms);
        }
    }

    send_log(&tx, "[-] durduruldu");
    Ok(())
}

fn sleep_interruptible(running: &AtomicBool, ms: u64) {
    let chunks = ms / 50;
    let rem = ms % 50;
    for _ in 0..chunks {
        if !running.load(Ordering::Acquire) { return; }
        thread::sleep(Duration::from_millis(50));
    }
    if rem > 0 && running.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(rem));
    }
}
