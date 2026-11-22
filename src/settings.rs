//! 設定 GUI: 起動時に設定ウィンドウを表示し、sparta.json の値を編集する。
//! Windows 専用。--console 指定時は従来のコンソール対話を使う。
//!
//! 戻り値: None = GUI が使えない (コンソール対話へフォールバック)
//!         Some(true) = 「開始」が押された (cfg は更新済み)
//!         Some(false) = キャンセルされた (終了する)

use serde_json::Value;
use std::collections::HashMap;

#[cfg(not(windows))]
pub fn show(
    _cfg: &mut Value,
    _bmz_label: &str,
    _tables: &[(i64, String)],
    _levels_map: &HashMap<i64, Vec<String>>,
) -> Option<bool> {
    None
}

#[cfg(windows)]
pub fn show(
    cfg: &mut Value,
    bmz_label: &str,
    tables: &[(i64, String)],
    levels_map: &HashMap<i64, Vec<String>>,
) -> Option<bool> {
    win::show(cfg, bmz_label, tables, levels_map)
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::config::{cfg_bool, cfg_i64, cfg_set, cfg_str};
    use serde_json::json;
    use std::cell::RefCell;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const ID_TABLE: i32 = 101;
    const ID_LEVEL: i32 = 102;
    const ID_MINUTES: i32 = 103;
    const ID_RESULT_SEC: i32 = 104;
    const ID_MODE: i32 = 105;
    const ID_GAUGE: i32 = 106;
    const ID_GAS: i32 = 107;
    const ID_HUD: i32 = 108;
    const ID_REPORT: i32 = 109;
    const ID_START: i32 = 1;
    const ID_CANCEL: i32 = 2;

    const WM_COMMAND: u32 = 0x0111;
    const WM_CREATE: u32 = 0x0001;
    const WM_CLOSE: u32 = 0x0010;
    const WM_DESTROY: u32 = 0x0002;
    const WM_SETFONT_MSG: u32 = 0x0030;
    const CB_ADDSTRING: u32 = 0x0143;
    const CB_RESETCONTENT: u32 = 0x014B;
    const CB_SETCURSEL: u32 = 0x014E;
    const CB_GETCURSEL: u32 = 0x0147;
    const BM_GETCHECK: u32 = 0x00F0;
    const BM_SETCHECK: u32 = 0x00F1;
    const CBN_SELCHANGE: u32 = 1;

    /// (sparta.json / bmz CLI の値, 表示名)
    const GAUGES: [(&str, &str); 6] = [
        ("assist-easy", "ASSIST EASY"),
        ("easy", "EASY"),
        ("normal", "NORMAL"),
        ("hard", "HARD"),
        ("ex-hard", "EX HARD"),
        ("hazard", "HAZARD"),
    ];

    /// 既定は best-clear。OFF 以外が選ばれている間はゲージをグレーアウトする
    const GASES: [(&str, &str); 5] = [
        ("off", "OFF"),
        ("continue", "CONTINUE"),
        ("hard-to-groove", "HARD-TO-GROOVE"),
        ("best-clear", "BEST CLEAR"),
        ("select-to-under", "SELECT-TO-UNDER"),
    ];

    const MODES: [(&str, &str); 3] = [
        ("priority", "未プレイ・未クリア優先"),
        ("noplay_failed", "未プレイ・FAILED優先"),
        ("random", "ランダム"),
    ];

    struct State {
        cfg: Value,
        bmz_label: String,
        tables: Vec<(i64, String)>,
        levels_map: HashMap<i64, Vec<String>>,
        h_table: HWND,
        h_level: HWND,
        h_minutes: HWND,
        h_result_sec: HWND,
        h_mode: HWND,
        h_gauge: HWND,
        h_gas: HWND,
        h_hud: HWND,
        h_report: HWND,
        result: Option<bool>,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    unsafe fn send(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> LRESULT {
        SendMessageW(hwnd, msg, WPARAM(wp), LPARAM(lp))
    }

    unsafe fn combo_add(hwnd: HWND, s: &str) {
        let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        send(hwnd, CB_ADDSTRING, 0, wide.as_ptr() as isize);
    }

    unsafe fn combo_sel(hwnd: HWND) -> i32 {
        send(hwnd, CB_GETCURSEL, 0, 0).0 as i32
    }

    unsafe fn get_text(hwnd: HWND) -> String {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    unsafe fn set_text(hwnd: HWND, s: &str) {
        let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
    }

    unsafe fn checked(hwnd: HWND) -> bool {
        send(hwnd, BM_GETCHECK, 0, 0).0 == 1
    }

    unsafe fn set_check(hwnd: HWND, on: bool) {
        send(hwnd, BM_SETCHECK, if on { 1 } else { 0 }, 0);
    }

    unsafe fn make(
        parent: HWND,
        class: PCWSTR,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: i32,
        font: HFONT,
    ) -> HWND {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let hinst = HINSTANCE(GetModuleHandleW(None).map(|m| m.0).unwrap_or_default());
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(wide.as_ptr()),
            WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
            x,
            y,
            w,
            h,
            parent,
            HMENU(id as *mut std::ffi::c_void),
            hinst,
            None,
        )
        .unwrap_or_default();
        if !hwnd.is_invalid() {
            send(hwnd, WM_SETFONT_MSG, font.0 as usize, 1);
        }
        hwnd
    }

    unsafe fn refill_levels(st: &State, table_id: i64, prefer: Option<&str>) {
        send(st.h_level, CB_RESETCONTENT, 0, 0);
        let levels = st.levels_map.get(&table_id);
        if let Some(levels) = levels {
            for lv in levels {
                combo_add(st.h_level, lv);
            }
            let idx = prefer
                .and_then(|p| levels.iter().position(|l| l == p))
                .unwrap_or(0);
            if !levels.is_empty() {
                send(st.h_level, CB_SETCURSEL, idx, 0);
            }
        }
    }

    unsafe fn create_controls(hwnd: HWND) {
        let font = CreateFontW(
            -14,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1, // DEFAULT_CHARSET
            0,
            0,
            4, // ANTIALIASED_QUALITY
            0,
            w!("MS Gothic"),
        );
        STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(st) = guard.as_mut() else { return };
            let label = |x: i32, y: i32, w: i32, text: &str| {
                make(hwnd, w!("STATIC"), text, 0, x, y + 3, w, 20, 0, font)
            };
            make(
                hwnd,
                w!("STATIC"),
                &format!("bmz-player: {}", st.bmz_label),
                0,
                12,
                12,
                440,
                20,
                0,
                font,
            );
            label(12, 44, 176, "難易度表");
            st.h_table = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0,
                196,
                44,
                248,
                200,
                ID_TABLE,
                font,
            );
            label(12, 82, 176, "開始レベル");
            st.h_level = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0,
                196,
                82,
                248,
                200,
                ID_LEVEL,
                font,
            );
            label(12, 120, 176, "セッション時間 (分)");
            st.h_minutes = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                120,
                90,
                24,
                ID_MINUTES,
                font,
            );
            label(12, 156, 176, "次の曲までの待機 (秒)");
            st.h_result_sec = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                156,
                90,
                24,
                ID_RESULT_SEC,
                font,
            );
            make(
                hwnd,
                w!("STATIC"),
                "(曲が終わった時点から数えます)",
                0,
                196,
                182,
                248,
                18,
                0,
                font,
            );
            label(12, 204, 176, "選曲方式");
            st.h_mode = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                204,
                248,
                200,
                ID_MODE,
                font,
            );
            label(12, 280, 176, "ゲージ");
            st.h_gauge = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                280,
                248,
                200,
                ID_GAUGE,
                font,
            );
            label(12, 242, 176, "GAS");
            st.h_gas = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                242,
                248,
                200,
                ID_GAS,
                font,
            );
            st.h_hud = make(
                hwnd,
                w!("BUTTON"),
                "HUDを表示する",
                BS_AUTOCHECKBOX as u32,
                196,
                318,
                248,
                22,
                ID_HUD,
                font,
            );
            st.h_report = make(
                hwnd,
                w!("BUTTON"),
                "終了時に結果HTMLを開く",
                BS_AUTOCHECKBOX as u32,
                196,
                344,
                248,
                22,
                ID_REPORT,
                font,
            );
            make(
                hwnd,
                w!("BUTTON"),
                "開始",
                BS_PUSHBUTTON as u32,
                196,
                384,
                110,
                34,
                ID_START,
                font,
            );
            make(
                hwnd,
                w!("BUTTON"),
                "キャンセル",
                BS_PUSHBUTTON as u32,
                334,
                384,
                110,
                34,
                ID_CANCEL,
                font,
            );

            // 初期値
            for (_, name) in &st.tables {
                combo_add(st.h_table, name);
            }
            let cur_table = cfg_i64(&st.cfg, "table_id");
            let t_idx = cur_table
                .and_then(|id| st.tables.iter().position(|(t, _)| *t == id))
                .unwrap_or(0);
            send(st.h_table, CB_SETCURSEL, t_idx, 0);
            let table_id = st.tables[t_idx].0;
            let start_level = cfg_str(&st.cfg, "start_level");
            refill_levels(st, table_id, start_level.as_deref());
            set_text(
                st.h_minutes,
                &cfg_i64(&st.cfg, "minutes").unwrap_or(60).to_string(),
            );
            set_text(
                st.h_result_sec,
                &cfg_i64(&st.cfg, "result_seconds").unwrap_or(20).to_string(),
            );
            for (_, name) in MODES {
                combo_add(st.h_mode, name);
            }
            let cur_mode = cfg_str(&st.cfg, "pick_mode").unwrap_or_else(|| "priority".into());
            let m_idx = MODES
                .iter()
                .position(|(k, _)| *k == cur_mode)
                .unwrap_or(0);
            send(st.h_mode, CB_SETCURSEL, m_idx, 0);
            for (_, name) in GAUGES {
                combo_add(st.h_gauge, name);
            }
            let cur_gauge = cfg_str(&st.cfg, "gauge").unwrap_or_default();
            let g_idx = GAUGES
                .iter()
                .position(|(k, _)| *k == cur_gauge)
                .unwrap_or(4); // 既定: EX HARD
            send(st.h_gauge, CB_SETCURSEL, g_idx, 0);
            for (_, name) in GASES {
                combo_add(st.h_gas, name);
            }
            let cur_gas = cfg_str(&st.cfg, "gas").unwrap_or_default();
            let gas_idx = GASES
                .iter()
                .position(|(k, _)| *k == cur_gas)
                .unwrap_or(3); // 既定: BEST CLEAR
            send(st.h_gas, CB_SETCURSEL, gas_idx, 0);
            update_gauge_enabled(st);
            set_check(st.h_hud, cfg_bool(&st.cfg, "hud", true));
            set_check(st.h_report, cfg_bool(&st.cfg, "report", true));
        });
    }

    /// GAS が有効 (OFF 以外) のときはゲージをグレーアウトする。
    /// 保存済みのゲージ値自体は保持する
    unsafe fn update_gauge_enabled(st: &State) {
        let gas_sel = combo_sel(st.h_gas);
        let gas_active = gas_sel >= 1;
        let _ = EnableWindow(st.h_gauge, !gas_active);
    }

    /// 「開始」: コントロール値を cfg へ回収する
    unsafe fn collect(st: &mut State) {
        let t_sel = combo_sel(st.h_table);
        if t_sel >= 0 && (t_sel as usize) < st.tables.len() {
            let (table_id, _) = st.tables[t_sel as usize];
            cfg_set(&mut st.cfg, "table_id", json!(table_id));
            let l_sel = combo_sel(st.h_level);
            if let Some(levels) = st.levels_map.get(&table_id) {
                if l_sel >= 0 && (l_sel as usize) < levels.len() {
                    cfg_set(
                        &mut st.cfg,
                        "start_level",
                        json!(levels[l_sel as usize].clone()),
                    );
                }
            }
        }
        let minutes: i64 = get_text(st.h_minutes).trim().parse().unwrap_or(60);
        cfg_set(&mut st.cfg, "minutes", json!(minutes.max(1)));
        let rs: i64 = get_text(st.h_result_sec).trim().parse().unwrap_or(20);
        cfg_set(&mut st.cfg, "result_seconds", json!(rs.max(0)));
        let m_sel = combo_sel(st.h_mode);
        if m_sel >= 0 && (m_sel as usize) < MODES.len() {
            cfg_set(&mut st.cfg, "pick_mode", json!(MODES[m_sel as usize].0));
        }
        let g_sel = combo_sel(st.h_gauge);
        if g_sel >= 0 && (g_sel as usize) < GAUGES.len() {
            cfg_set(&mut st.cfg, "gauge", json!(GAUGES[g_sel as usize].0));
        }
        let gas_sel = combo_sel(st.h_gas);
        if gas_sel >= 0 && (gas_sel as usize) < GASES.len() {
            cfg_set(&mut st.cfg, "gas", json!(GASES[gas_sel as usize].0));
        }
        cfg_set(&mut st.cfg, "hud", json!(checked(st.h_hud)));
        cfg_set(&mut st.cfg, "report", json!(checked(st.h_report)));
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                create_controls(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as i32;
                let code = ((wp.0 >> 16) & 0xffff) as usize;
                if id == ID_TABLE && code == CBN_SELCHANGE as usize {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            let sel = unsafe { combo_sel(st.h_table) };
                            if sel >= 0 && (sel as usize) < st.tables.len() {
                                let tid = st.tables[sel as usize].0;
                                unsafe { refill_levels(st, tid, None) };
                            }
                        }
                    });
                    LRESULT(0)
                } else if id == ID_GAS && code == CBN_SELCHANGE as usize {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { update_gauge_enabled(st) };
                        }
                    });
                    LRESULT(0)
                } else if id == ID_START {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { collect(st) };
                            st.result = Some(true);
                        }
                    });
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                } else if id == ID_CANCEL {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            st.result = Some(false);
                        }
                    });
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_CLOSE => {
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        if st.result.is_none() {
                            st.result = Some(false);
                        }
                    }
                });
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn show(
        cfg: &mut Value,
        bmz_label: &str,
        tables: &[(i64, String)],
        levels_map: &HashMap<i64, Vec<String>>,
    ) -> Option<bool> {
        if tables.is_empty() {
            return None;
        }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
        }
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                cfg: cfg.clone(),
                bmz_label: bmz_label.to_string(),
                tables: tables.to_vec(),
                levels_map: levels_map.clone(),
                h_table: HWND::default(),
                h_level: HWND::default(),
                h_minutes: HWND::default(),
                h_result_sec: HWND::default(),
                h_mode: HWND::default(),
                h_gauge: HWND::default(),
                h_gas: HWND::default(),
                h_hud: HWND::default(),
                h_report: HWND::default(),
                result: None,
            });
        });
        unsafe {
            let hinst = HINSTANCE(
                GetModuleHandleW(None)
                    .map(|m| m.0)
                    .unwrap_or_default(),
            );
            let class_name = w!("BmsSpartaSettings");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let title: Vec<u16> = "bmz-sparta 設定"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                480,
                480,
                None,
                None,
                hinst,
                None,
            );
            let hwnd = match hwnd {
                Ok(h) => h,
                Err(_) => return None,
            };
            let _ = ShowWindow(hwnd, SW_SHOW);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let st = STATE.with(|s| s.borrow_mut().take());
        match st {
            Some(st) => match st.result {
                Some(true) => {
                    *cfg = st.cfg;
                    Some(true)
                }
                Some(false) => Some(false),
                None => None,
            },
            None => None,
        }
    }
}
