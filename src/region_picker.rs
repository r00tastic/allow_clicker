//! Ekran uzerinde saydam overlay ile drag select bolge secici.
//! Win32 API kullanir. Ayri bir thread'de calisir, sonucu mpsc ile doner.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
};

use anyhow::{anyhow, Result};

use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FrameRect, HGDIOBJ,
            InvalidateRect, PAINTSTRUCT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW,
                SetForegroundWindow, SetLayeredWindowAttributes, ShowWindow, TranslateMessage,
                CS_HREDRAW, CS_VREDRAW, HCURSOR, IDC_CROSS, LWA_ALPHA, MSG, SM_CXVIRTUALSCREEN,
                SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW, WM_DESTROY,
                WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
                WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
    },
};

#[derive(Debug, Clone, Copy)]
pub struct SelectedRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

struct DragState {
    dragging: bool,
    start: POINT,
    curr: POINT,
    prev: POINT,
    result: Option<RECT>,
    origin_x: i32,
    origin_y: i32,
}

static DRAG: OnceLock<Mutex<Option<DragState>>> = OnceLock::new();
static PICKING: AtomicBool = AtomicBool::new(false);

fn drag_slot() -> &'static Mutex<Option<DragState>> {
    DRAG.get_or_init(|| Mutex::new(None))
}

/// Yeni bir thread'de overlay ac ve kullanicinin dikdortgen secmesini bekle.
/// Cancel: ESC. Ust uste cagri korunur (PICKING guard).
pub fn pick_region_async(tx: mpsc::Sender<Option<SelectedRect>>) {
    if PICKING.swap(true, Ordering::AcqRel) {
        // Zaten bir picker aciksa iptal (bir seyi bozmayalim)
        let _ = tx.send(None);
        return;
    }
    thread::spawn(move || {
        let result = run_overlay();
        PICKING.store(false, Ordering::Release);
        let _ = tx.send(result.ok().flatten());
    });
}

fn run_overlay() -> Result<Option<SelectedRect>> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();

        let class_name = w!("AllowClickerRegionPicker");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(None, IDC_CROSS).unwrap_or(HCURSOR::default());
        wc.lpszClassName = class_name;
        // Ayni ismi tekrar register etmek hata verse de umursamiyoruz (once yaptiysak var)
        RegisterClassW(&wc);

        let sx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let sy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let sw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let sh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("Bolge Sec (ESC ile iptal)"),
            WS_POPUP | WS_VISIBLE,
            sx,
            sy,
            sw,
            sh,
            None,
            None,
            Some(hinst),
            None,
        )
        .map_err(|e| anyhow!("CreateWindowExW: {}", e))?;

        // ~%30 opak siyah — arka planı hafif karartır, seçim görünsün
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 76, LWA_ALPHA);

        {
            let mut slot = drag_slot().lock().unwrap();
            *slot = Some(DragState {
                dragging: false,
                start: POINT::default(),
                curr: POINT::default(),
                prev: POINT::default(),
                result: None,
                origin_x: sx,
                origin_y: sy,
            });
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));

        // Message loop — -1 hata durumunu da handle et
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let slot = drag_slot().lock().unwrap();
        if let Some(state) = slot.as_ref() {
            if let Some(r) = state.result {
                let x = r.left.min(r.right);
                let y = r.top.min(r.bottom);
                let w = (r.right - r.left).abs();
                let h = (r.bottom - r.top).abs();
                if w >= 4 && h >= 4 {
                    return Ok(Some(SelectedRect { x, y, w, h }));
                }
            }
        }
        Ok(None)
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Kendimiz cizecegiz (layered pencerede flicker olmasin)
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut slot = drag_slot().lock().unwrap();
            if let Some(s) = slot.as_mut() {
                s.dragging = true;
                s.start = POINT { x, y };
                s.curr = POINT { x, y };
                s.prev = POINT { x, y };
            }
            drop(slot);
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut slot = drag_slot().lock().unwrap();
            let mut old_rect: Option<RECT> = None;
            let mut new_rect: Option<RECT> = None;
            if let Some(s) = slot.as_mut() {
                if s.dragging {
                    old_rect = Some(rect_of(s.start, s.prev));
                    s.curr = POINT { x, y };
                    new_rect = Some(rect_of(s.start, s.curr));
                    s.prev = s.curr;
                }
            }
            drop(slot);
            if let Some(r) = old_rect {
                let inflated = inflate(r, 2);
                let _ = InvalidateRect(Some(hwnd), Some(&inflated), false);
            }
            if let Some(r) = new_rect {
                let inflated = inflate(r, 2);
                let _ = InvalidateRect(Some(hwnd), Some(&inflated), false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut slot = drag_slot().lock().unwrap();
            if let Some(s) = slot.as_mut() {
                if s.dragging {
                    s.dragging = false;
                    let left = s.origin_x + s.start.x.min(x);
                    let top = s.origin_y + s.start.y.min(y);
                    let right = s.origin_x + s.start.x.max(x);
                    let bottom = s.origin_y + s.start.y.max(y);
                    s.result = Some(RECT { left, top, right, bottom });
                }
            }
            drop(slot);
            let _ = ReleaseCapture();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == 0x1B {
                // ESC — iptal
                let mut slot = drag_slot().lock().unwrap();
                if let Some(s) = slot.as_mut() {
                    s.result = None;
                }
                drop(slot);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let slot = drag_slot().lock().unwrap();
            if let Some(s) = slot.as_ref() {
                if s.dragging {
                    let r = rect_of(s.start, s.curr);
                    let brush = CreateSolidBrush(COLORREF(0x00FF00)); // yesil BGR
                    let _ = FrameRect(hdc, &r, brush);
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
            }
            drop(slot);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn rect_of(a: POINT, b: POINT) -> RECT {
    RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}

fn inflate(r: RECT, d: i32) -> RECT {
    RECT {
        left: r.left - d,
        top: r.top - d,
        right: r.right + d,
        bottom: r.bottom + d,
    }
}
