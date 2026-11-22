//! スパルタ HUD: sparta-status.json を読んで最前面に状態を表示する。
//! Win32 レイヤードウィンドウ (UpdateLayeredWindow) でピクセル単位の透過。
//! 背景は黒の極薄 (alpha 26/255)、文字・バーは不透明。
//! 位置・サイズは hud-pos.json の矩形で FIX (入力は受けない)。
//!
//! 描画は GDI で DIB へ行い、最後にアルファを整える
//! (黒でない画素 = 不透明、黒 = 背景アルファ)。

use serde_json::Value;

#[cfg(windows)]
pub fn now_secs() -> f64 {
    crate::session::now_secs()
}

#[cfg(windows)]
pub fn read_status() -> Option<Value> {
    let text = std::fs::read_to_string(crate::config::status_path()).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

#[cfg(not(windows))]
pub fn run_hud() -> i32 {
    eprintln!("HUD は Windows 専用です。");
    1
}

#[cfg(windows)]
pub fn run_hud() -> i32 {
    win::run()
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const BG_ALPHA: u8 = 26;
    const FG: (u8, u8, u8) = (245, 245, 245);
    const SUB: (u8, u8, u8) = (168, 168, 168);
    const ACCENT: (u8, u8, u8) = (253, 29, 29);
    const RED: (u8, u8, u8) = (255, 48, 64);
    const BAR_BG: (u8, u8, u8) = (38, 38, 38);
    const BASE_W: f64 = 460.0;

    fn colorref(c: (u8, u8, u8)) -> COLORREF {
        COLORREF(c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16)
    }

    struct Fonts {
        mid: HFONT,
        legend: HFONT,
        lh: i32,
        legend_step: i32,
        pad: i32,
        top: i32,
        bar_h: i32,
    }

    struct Renderer {
        w: i32,
        h: i32,
        s: f64,
        fonts: HashMap<i32, Fonts>,
    }

    impl Renderer {
        fn new(w: i32, h: i32) -> Renderer {
            let s = (w as f64 / BASE_W).clamp(0.8, 1.7);
            Renderer {
                w,
                h,
                s,
                fonts: HashMap::new(),
            }
        }

        fn fonts_for(&mut self, s: f64) -> &Fonts {
            let key = (s * 1000.0) as i32;
            self.fonts.entry(key).or_insert_with(|| unsafe {
                let px = |v: f64| -> i32 { ((v * s) as i32).max(9) };
                let make = |size: i32| -> HFONT {
                    CreateFontW(
                        -size,
                        0,
                        0,
                        0,
                        400,
                        0, // italic
                        0, // underline
                        0, // strikeout
                        1, // DEFAULT_CHARSET
                        0, // OUT_DEFAULT_PRECIS
                        0, // CLIP_DEFAULT_PRECIS
                        4, // ANTIALIASED_QUALITY
                        0, // DEFAULT_PITCH
                        w!("MS Gothic"),
                    )
                };
                Fonts {
                    mid: make(px(17.0)),
                    legend: make(px(13.0)),
                    lh: (24.0 * s) as i32,
                    legend_step: (19.0 * s) as i32,
                    pad: (18.0 * s) as i32,
                    top: (12.0 * s) as i32,
                    bar_h: (6.0 * s) as i32,
                }
            })
        }

        unsafe fn text(
            &self,
            dc: HDC,
            x: i32,
            y: i32,
            s: &str,
            font: HFONT,
            color: (u8, u8, u8),
            right: bool,
            pad: i32,
        ) {
            if s.is_empty() {
                return;
            }
            let wide: Vec<u16> = s.encode_utf16().collect();
            let old = SelectObject(dc, HGDIOBJ(font.0));
            SetTextColor(dc, colorref(color));
            let mut x = x;
            if right {
                let mut sz = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &wide, &mut sz);
                x = self.w - pad - sz.cx;
            }
            let _ = TextOutW(dc, x, y, &wide);
            SelectObject(dc, old);
        }

        unsafe fn round_bar(
            &self,
            dc: HDC,
            x1: i32,
            y1: i32,
            x2: i32,
            y2: i32,
            color: (u8, u8, u8),
        ) {
            if x2 <= x1 {
                return;
            }
            let brush = CreateSolidBrush(colorref(color));
            let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
            let null_pen = GetStockObject(NULL_PEN);
            let old_pen = SelectObject(dc, null_pen);
            let d = (y2 - y1).max(1);
            let _ = RoundRect(dc, x1, y1, x2, y2, d, d);
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }

        /// 状態を DC へ描画する
        fn render(&mut self, dc: HDC, st: &Value) {
            // (text, color, right_text)
            let mut lines: Vec<(String, (u8, u8, u8), Option<String>)> = Vec::new();
            let ended = st.get("state").and_then(|v| v.as_str()) == Some("ended");
            let result = st.get("state").and_then(|v| v.as_str()) == Some("result");
            let get_i = |k: &str| st.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
            let get_s = |k: &str| {
                st.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            if ended {
                lines.push(("END".into(), FG, None));
                lines.push((
                    format!("{}曲プレイ / クリア {}", get_i("song"), get_i("cleared")),
                    SUB,
                    None,
                ));
                lines.push(("おつかれさまでした".into(), SUB, None));
            } else {
                let remain = st.get("deadline").and_then(|v| v.as_i64()).map(|d| {
                    let sec = (d - now_secs() as i64).max(0);
                    format!("残り {}:{:02}", sec / 60, sec % 60)
                });
                lines.push((
                    format!("現在のターゲット　{}", get_s("table")),
                    FG,
                    remain,
                ));
                let done = (get_i("song") - 1).max(0);
                lines.push((
                    get_s("level"),
                    FG,
                    Some(format!(
                        "{}曲目 · クリア {}/{}",
                        get_i("song"),
                        get_i("cleared"),
                        done
                    )),
                ));
                lines.push((String::new(), FG, None));
                // ベスト/結果ブロックは常に4行固定 (縦位置をずらさない)
                if result && !get_s("last_lamp").is_empty() {
                    let lamp = get_s("last_lamp");
                    let col = if lamp == "FAILED" { RED } else { FG };
                    let mut combo = lamp.clone();
                    if !get_s("last_score_fmt").is_empty() {
                        combo += &format!("　{}", get_s("last_score_fmt"));
                    }
                    combo += &format!(
                        "　BP {}",
                        st.get("last_bp")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "null".into())
                    );
                    if get_i("last_gain") != 0 {
                        combo += &format!("　+{}pt", get_i("last_gain"));
                    }
                    lines.push(("結果".into(), ACCENT, None));
                    lines.push((combo, col, None));
                    lines.push((String::new(), FG, None));
                    lines.push((String::new(), FG, None));
                } else {
                    lines.push(("この曲のベスト".into(), FG, None));
                    if !get_s("prev_lamp").is_empty() {
                        let mut combo = get_s("prev_lamp");
                        if !get_s("prev_score_fmt").is_empty() {
                            combo += &format!("　{}", get_s("prev_score_fmt"));
                        }
                        combo += &format!(
                            "　BP {}",
                            st.get("prev_bp")
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "null".into())
                        );
                        lines.push((combo, FG, None));
                    } else {
                        lines.push(("NO PLAY".into(), FG, None));
                    }
                    lines.push((String::new(), FG, None));
                    lines.push((String::new(), FG, None));
                }
                lines.push((String::new(), FG, None));
                let pts = get_i("points");
                let mx = if st.get("up_points").is_some() {
                    get_i("up_points")
                } else {
                    3
                };
                let lose = get_i("lose");
                let lose_max = if st.get("lose_max").is_some() {
                    get_i("lose_max")
                } else {
                    3
                };
                let seg_up = "■".repeat(pts.max(0) as usize)
                    + &"□".repeat((mx - pts).max(0) as usize);
                let seg_dn = "■".repeat(lose.max(0) as usize)
                    + &"□".repeat((lose_max - lose).max(0) as usize);
                lines.push((
                    format!("レベルアップ [{seg_up}]"),
                    FG,
                    Some(format!("あと {} pt", (mx - pts).max(0))),
                ));
                lines.push((
                    format!("レベルダウン [{seg_dn}]"),
                    if lose >= lose_max - 1 { RED } else { FG },
                    Some(format!("あと {} 回", (lose_max - lose).max(0))),
                ));
                lines.push((String::new(), FG, None));
            }
            let countdown = if !ended && result {
                st.get("skip_deadline").and_then(|v| v.as_i64()).map(|d| {
                    let left = (d as f64 - now_secs()).max(0.0);
                    format!("次の曲まで あと {} 秒", (left + 0.999) as i64)
                })
            } else {
                None
            };

            // 高さに収まるスケールを選ぶ (状態が変わっても原則一定)
            const BUDGET_UNITS: f64 = 12.0 + 24.0 * 11.0 + 19.0 * 4.0 + 32.0;
            let s_fit = if ended {
                self.s
            } else {
                self.s.min((self.h as f64 - 6.0) / BUDGET_UNITS)
            };
            let (mid, legend, lh, legend_step, pad, top, bar_h) = {
                let f = self.fonts_for(s_fit);
                (f.mid, f.legend, f.lh, f.legend_step, f.pad, f.top, f.bar_h)
            };
            unsafe {
                let _ = SetBkMode(dc, TRANSPARENT);
                let mut y = top;
                for (text, col, right) in &lines {
                    if !text.is_empty() {
                        self.text(dc, pad, y, text, mid, *col, false, pad);
                    }
                    if let Some(r) = right {
                        self.text(dc, 0, y, r, mid, FG, true, pad);
                    }
                    y += lh;
                }
                if !ended {
                    for line in [
                        "+3pt　EX HARD CLEAR以上",
                        "+2pt　HARD CLEAR",
                        "+1pt　CLEAR / EASY CLEAR",
                        "3pt でレベルアップ ／ 3連敗でレベルダウン",
                    ] {
                        self.text(dc, pad, y, line, legend, SUB, false, pad);
                        y += legend_step;
                    }
                }
                if let Some(cd) = &countdown {
                    self.text(dc, pad, y, cd, mid, ACCENT, false, pad);
                    y += lh;
                    let total = st.get("skip_total").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let left = st
                        .get("skip_deadline")
                        .and_then(|v| v.as_f64())
                        .map(|d| (d - now_secs()).max(0.0))
                        .unwrap_or(0.0);
                    let frac = if total > 0.0 { left / total } else { 0.0 };
                    self.round_bar(dc, pad, y, self.w - pad, y + bar_h, BAR_BG);
                    let w2 = ((self.w - 2 * pad) as f64 * frac) as i32;
                    if w2 > 0 {
                        self.round_bar(dc, pad, y, pad + w2, y + bar_h, ACCENT);
                    }
                }
            }
        }
    }

    struct HudState {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        renderer: Renderer,
        ended_at: Option<f64>,
    }

    thread_local! {
        static STATE: RefCell<Option<HudState>> = const { RefCell::new(None) };
    }

    /// 描画して表示。終了すべきなら true
    fn update(hwnd: HWND) -> bool {
        let st = read_status();
        if let Some(st) = &st {
            if st.get("state").and_then(|v| v.as_str()) == Some("ended") {
                let mut close = false;
                STATE.with(|s| {
                    if let Some(h) = s.borrow_mut().as_mut() {
                        if h.ended_at.is_none() {
                            h.ended_at = Some(now_secs());
                        } else if now_secs() - h.ended_at.unwrap() > 5.0 {
                            close = true;
                        }
                    }
                });
                if close {
                    return true;
                }
            }
            blit(hwnd, st);
        }
        false
    }

    fn blit(hwnd: HWND, st: &Value) {
        STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(h) = guard.as_mut() else { return };
            let (w, hh) = (h.w, h.h);
            unsafe {
                let screen_dc = GetDC(None);
                let mem_dc = CreateCompatibleDC(screen_dc);
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -hh,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [RGBQUAD::default(); 1],
                };
                let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let hbmp = CreateDIBSection(
                    mem_dc,
                    &bmi,
                    DIB_RGB_COLORS,
                    &mut ptr,
                    None,
                    0,
                )
                .unwrap_or_default();
                if hbmp.is_invalid() || ptr.is_null() {
                    let _ = DeleteDC(mem_dc);
                    ReleaseDC(None, screen_dc);
                    return;
                }
                // 背景: 黒 + alpha 26 で初期化
                let pixels = std::slice::from_raw_parts_mut(
                    ptr as *mut u8,
                    (w * hh * 4) as usize,
                );
                for px in pixels.chunks_exact_mut(4) {
                    px[0] = 0;
                    px[1] = 0;
                    px[2] = 0;
                    px[3] = BG_ALPHA;
                }
                let old = SelectObject(mem_dc, HGDIOBJ(hbmp.0));
                h.renderer.render(mem_dc, st);
                // アルファ整形: 黒でない画素は不透明、黒は背景アルファ
                for px in pixels.chunks_exact_mut(4) {
                    if px[0] != 0 || px[1] != 0 || px[2] != 0 {
                        px[3] = 255;
                    } else {
                        px[3] = BG_ALPHA;
                    }
                }
                let dst = POINT { x: h.x, y: h.y };
                let src = POINT { x: 0, y: 0 };
                let size = SIZE { cx: w, cy: hh };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    screen_dc,
                    Some(std::ptr::addr_of!(dst)),
                    Some(std::ptr::addr_of!(size)),
                    mem_dc,
                    Some(std::ptr::addr_of!(src)),
                    COLORREF(0),
                    Some(std::ptr::addr_of!(blend)),
                    ULW_ALPHA,
                );
                SelectObject(mem_dc, old);
                let _ = DeleteObject(HGDIOBJ(hbmp.0));
                let _ = DeleteDC(mem_dc);
                ReleaseDC(None, screen_dc);
            }
        });
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        const WM_TIMER: u32 = 0x0113;
        const WM_DESTROY: u32 = 0x0002;
        match msg {
            WM_TIMER => {
                if update(hwnd) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn run() -> i32 {
        let saved: Value = std::fs::read_to_string(crate::config::hud_pos_path())
            .ok()
            .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or(Value::Null);
        let gi = |k: &str, d: i32| saved.get(k).and_then(|v| v.as_i64()).unwrap_or(d as i64) as i32;
        let (x, y, w, h) = (gi("x", 1180), gi("y", 310), gi("w", 460), gi("h", 400));
        STATE.with(|s| {
            *s.borrow_mut() = Some(HudState {
                x,
                y,
                w,
                h,
                renderer: Renderer::new(w, h),
                ended_at: None,
            });
        });
        unsafe {
            let hinst: HINSTANCE = GetModuleHandleW(None)
                .map(|m| HINSTANCE(m.0))
                .unwrap_or_default();
            let class_name = w!("BmsSpartaHud");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let ex = WS_EX_LAYERED
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE;
            let hwnd = CreateWindowExW(
                ex,
                class_name,
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                hinst,
                None,
            );
            let hwnd = match hwnd {
                Ok(h) => h,
                Err(_) => return 1,
            };
            let _ = ShowWindow(hwnd, SW_SHOW);
            SetTimer(hwnd, 1, 500, None);
            update(hwnd);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        0
    }
}
