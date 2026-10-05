use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{anyhow, Result};
use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
use windows::Win32::{
    Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFOHEADER, DIB_RGB_COLORS,
        SRCCOPY,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

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

/// GDI BitBlt ile ekrandan belirli bir bolgeyi yakala.
/// CPU-only — GPU kullanmaz. Piksel verisi BGRA formatinda doner.
fn capture_screen(x: i32, y: i32, w: i32, h: i32, buf: &mut Vec<u8>) -> Result<()> {
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return Err(anyhow!("GetDC failed"));
        }
        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        if hdc_mem.is_invalid() {
            ReleaseDC(None, hdc_screen);
            return Err(anyhow!("CreateCompatibleDC failed"));
        }
        let hbm = CreateCompatibleBitmap(hdc_screen, w, h);
        if hbm.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            return Err(anyhow!("CreateCompatibleBitmap failed"));
        }
        let old = SelectObject(hdc_mem, hbm.into());
        let _ = BitBlt(hdc_mem, 0, 0, w, h, Some(hdc_screen), x, y, SRCCOPY);

        let mut bmi_header: BITMAPINFOHEADER = std::mem::zeroed();
        bmi_header.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi_header.biWidth = w;
        bmi_header.biHeight = -h; // top-down
        bmi_header.biPlanes = 1;
        bmi_header.biBitCount = 32;

        let expected = (w as usize) * (h as usize) * 4;
        buf.resize(expected, 0);

        GetDIBits(
            hdc_mem,
            hbm,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bmi_header as *mut _ as *mut _,
            DIB_RGB_COLORS,
        );

        // GDI alpha=0 verir, 255'e set et
        for px in buf.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbm.into());
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        Ok(())
    }
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

    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            send_log(&tx, format!("[HATA] Enigo: {}", e));
            return Ok(());
        }
    };

    // Yakalama bolgesi: ayarlanmissa region, yoksa birincil monitor
    let (cap_x, cap_y, cap_w, cap_h) = if cfg.region_w > 0 && cfg.region_h > 0 {
        (cfg.region_x, cfg.region_y, cfg.region_w as i32, cfg.region_h as i32)
    } else {
        unsafe {
            (0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        }
    };

    send_log(&tx, format!(
        "[+] baslatildi — hedefler={:?}, bolge=({},{},{},{}){}",
        cfg.targets, cap_x, cap_y, cap_w, cap_h,
        if cfg.dry_run { " [DRY-RUN]" } else { "" }
    ));

    let mut bgra_buf: Vec<u8> = Vec::with_capacity((cap_w as usize) * (cap_h as usize) * 4);

    while running.load(Ordering::Acquire) {
        if let Err(e) = capture_screen(cap_x, cap_y, cap_w, cap_h, &mut bgra_buf) {
            send_log(&tx, format!("[!] capture: {}", e));
            thread::sleep(Duration::from_millis(1000));
            continue;
        }

        let hit: Option<Hit> = match ocr.find_any(
            &bgra_buf,
            cap_w as u32,
            cap_h as u32,
            &cfg.targets,
            cfg.case_sensitive,
            cap_x,
            cap_y,
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
