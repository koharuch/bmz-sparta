//! 設定 GUI: 起動時に設定ウィンドウを表示し、sparta.json の値を編集する。
//! Windows 専用。--console 指定時は従来のコンソール対話を使う。
//!
//! 戻り値: None = GUI が使えない (コンソール対話へフォールバック)
//!         Some((true, persist)) = 「開始」。persist は設定を保存してよいか
//!         Some((false, _)) = キャンセルされた (終了する)

use serde_json::Value;
use std::collections::HashMap;

#[cfg(not(windows))]
pub fn show(
    _cfg: &mut Value,
    _bmz_label: &str,
    _tables: &[(i64, String)],
    _levels_map: &HashMap<i64, Vec<String>>,
) -> Option<(bool, bool)> {
    None
}

#[cfg(windows)]
pub fn show(
    cfg: &mut Value,
    bmz_label: &str,
    tables: &[(i64, String)],
    levels_map: &HashMap<i64, Vec<String>>,
) -> Option<(bool, bool)> {
    win::show(cfg, bmz_label, tables, levels_map)
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::config::{cfg_bool, cfg_i64, cfg_set, cfg_str};
    use serde_json::json;
    use std::cell::RefCell;
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::UI::Controls::{NMHDR, TCITEMHEADERA_MASK, TCITEMW};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };
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
    const ID_DAY_START: i32 = 110;
    const ID_TAB: i32 = 111;
    const ID_BROWSE: i32 = 112;
    const ID_HUD_ANCHOR: i32 = 113;
    const ID_HUD_WIDTH: i32 = 114;
    const ID_HUD_ALPHA: i32 = 115;
    const ID_HUD_BEST: i32 = 116;
    const ID_HUD_LEGEND: i32 = 117;
    const ID_HUD_REMAIN: i32 = 118;
    const ID_HUD_COUNTDOWN: i32 = 119;
    const ID_UP_POINTS: i32 = 120;
    const ID_PTS_EXHARD: i32 = 121;
    const ID_PTS_HARD: i32 = 122;
    const ID_PTS_CLEAR: i32 = 123;
    const ID_FAIL_STREAK: i32 = 124;
    const ID_SAVE: i32 = 125;
    const ID_PREVIEW: i32 = 126;
    const WM_TIMER: u32 = 0x0113;
    const PREVIEW_TIMER_ID: usize = 1;
    const ID_START: i32 = 1;

    const WM_COMMAND: u32 = 0x0111;
    const WM_NOTIFY: u32 = 0x004E;
    const WM_CTLCOLORBTN: u32 = 0x0135;
    const WM_CTLCOLORSTATIC: u32 = 0x0138;
    const TCM_INSERTITEMW: u32 = 0x133E; // TCM_FIRST(0x1300) + 62 (A版は +7)
    const TCM_GETCURSEL: u32 = 0x130B;
    const TCN_SELCHANGE: u32 = 4294966745; // -551
    const TCIF_TEXT: u32 = 0x0001;
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
        ("priority", "NO PLAY 優先"),
        ("noplay_failed", "NO PLAY + FAILED 優先"),
        ("random", "ランダム"),
    ];

    /// HUD の表示位置プリセット (sparta.json の hud_anchor 値, 表示名)
    const HUD_ANCHORS: [(&str, &str); 5] = [
        ("custom", "カスタム (hud-pos.json)"),
        ("tl", "左上"),
        ("tr", "右上"),
        ("bl", "左下"),
        ("br", "右下"),
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
        h_day_start: HWND,
        h_mode: HWND,
        h_gauge: HWND,
        h_gas: HWND,
        h_hud: HWND,
        h_report: HWND,
        h_tab: HWND,
        h_bmz_label: HWND,
        h_anchor: HWND,
        h_hud_w: HWND,
        h_hud_alpha: HWND,
        h_best: HWND,
        h_legend: HWND,
        h_remain: HWND,
        h_countdown: HWND,
        h_up_points: HWND,
        h_pts_exhard: HWND,
        h_pts_hard: HWND,
        h_pts_clear: HWND,
        h_fail_streak: HWND,
        h_preview: HWND,
        h_note: HWND,
        preview: Option<std::process::Child>,
        bmz_child: Option<std::process::Child>,
        preview_written: String,
        pos_seen: String,
        page_main: Vec<HWND>,
        page_report: Vec<HWND>,
        page_hud: Vec<HWND>,
        page_adv: Vec<HWND>,
        result: Option<(bool, bool)>,
        saved_cfg: Value,
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
            -15,
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
            w!("Yu Gothic UI"),
        );
        STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(st) = guard.as_mut() else { return };
            let label = |page: &mut Vec<HWND>, x: i32, y: i32, w: i32, text: &str| {
                page.push(make(hwnd, w!("STATIC"), text, 0, x, y + 3, w, 20, 0, font));
            };
            st.h_bmz_label = make(
                hwnd,
                w!("STATIC"),
                &format!("bmz-player: {}", st.bmz_label),
                0,
                12,
                14,
                348,
                20,
                0,
                font,
            );
            make(
                hwnd,
                w!("BUTTON"),
                "参照...",
                BS_PUSHBUTTON as u32,
                368,
                9,
                84,
                26,
                ID_BROWSE,
                font,
            );
            // タブ: 「プレイ」= セッション設定、「レポート」= HTML出力まわり
            st.h_tab = make(
                hwnd,
                w!("SysTabControl32"),
                "",
                0,
                6,
                40,
                452,
                384,
                ID_TAB,
                font,
            );
            for (i, name) in ["プレイ", "レポート", "HUD", "詳細"].iter().enumerate() {
                let mut text: Vec<u16> =
                    name.encode_utf16().chain(std::iter::once(0)).collect();
                let item = TCITEMW {
                    mask: TCITEMHEADERA_MASK(TCIF_TEXT),
                    pszText: PWSTR(text.as_mut_ptr()),
                    ..Default::default()
                };
                send(st.h_tab, TCM_INSERTITEMW, i, &item as *const TCITEMW as isize);
            }
            label(&mut st.page_main, 12, 80, 176, "難易度表");
            st.h_table = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0,
                196,
                80,
                248,
                200,
                ID_TABLE,
                font,
            );
            label(&mut st.page_main, 12, 118, 176, "開始レベル");
            st.h_level = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0,
                196,
                118,
                248,
                200,
                ID_LEVEL,
                font,
            );
            label(&mut st.page_main, 12, 156, 176, "セッション時間 (分)");
            st.h_minutes = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                156,
                90,
                24,
                ID_MINUTES,
                font,
            );
            label(&mut st.page_main, 12, 192, 176, "インターバル (秒)");
            st.h_result_sec = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                192,
                90,
                24,
                ID_RESULT_SEC,
                font,
            );
            label(&mut st.page_report, 12, 124, 176, "日付の切り替わり (時)");
            st.h_day_start = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                124,
                90,
                24,
                ID_DAY_START,
                font,
            );
            label(&mut st.page_main, 12, 264, 176, "選曲方式");
            st.h_mode = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                264,
                248,
                200,
                ID_MODE,
                font,
            );
            label(&mut st.page_main, 12, 340, 176, "ゲージ");
            st.h_gauge = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                340,
                248,
                200,
                ID_GAUGE,
                font,
            );
            label(&mut st.page_main, 12, 302, 176, "GAS");
            st.h_gas = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                302,
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
                84,
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
                84,
                248,
                22,
                ID_REPORT,
                font,
            );
            // ---- HUDタブ ----
            label(&mut st.page_hud, 12, 124, 176, "表示位置");
            st.h_anchor = make(
                hwnd,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32,
                196,
                124,
                248,
                200,
                ID_HUD_ANCHOR,
                font,
            );
            label(&mut st.page_hud, 12, 162, 176, "幅 (px)");
            st.h_hud_w = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                162,
                90,
                24,
                ID_HUD_WIDTH,
                font,
            );
            label(&mut st.page_hud, 12, 200, 176, "背景の濃さ (0-255)");
            st.h_hud_alpha = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                200,
                90,
                24,
                ID_HUD_ALPHA,
                font,
            );
            st.h_best = make(
                hwnd,
                w!("BUTTON"),
                "この曲のベスト・結果を表示",
                BS_AUTOCHECKBOX as u32,
                196,
                230,
                248,
                22,
                ID_HUD_BEST,
                font,
            );
            st.h_legend = make(
                hwnd,
                w!("BUTTON"),
                "ルール凡例を表示",
                BS_AUTOCHECKBOX as u32,
                196,
                254,
                248,
                22,
                ID_HUD_LEGEND,
                font,
            );
            st.h_remain = make(
                hwnd,
                w!("BUTTON"),
                "残り時間を表示",
                BS_AUTOCHECKBOX as u32,
                196,
                278,
                248,
                22,
                ID_HUD_REMAIN,
                font,
            );
            st.h_countdown = make(
                hwnd,
                w!("BUTTON"),
                "次の曲までのカウントダウンを表示",
                BS_AUTOCHECKBOX as u32,
                196,
                302,
                248,
                22,
                ID_HUD_COUNTDOWN,
                font,
            );
            st.h_preview = make(
                hwnd,
                w!("BUTTON"),
                "プレビュー表示",
                BS_PUSHBUTTON as u32,
                196,
                332,
                248,
                30,
                ID_PREVIEW,
                font,
            );
            st.h_note = make(
                hwnd,
                w!("STATIC"),
                "自動再生でランダムな譜面が流れます（スコア保存なし）\r\nHUD はドラッグで移動でき、位置は記憶されます\r\n終了は「プレビュー終了」ボタン、または HUD で Esc",
                0,
                24,
                368,
                424,
                54,
                0,
                font,
            );
            // ---- 詳細タブ (レベル昇降ルール) ----
            label(&mut st.page_adv, 12, 84, 176, "レベルアップに必要な pt");
            st.h_up_points = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                84,
                90,
                24,
                ID_UP_POINTS,
                font,
            );
            label(&mut st.page_adv, 12, 122, 176, "EX HARD CLEAR 以上の pt");
            st.h_pts_exhard = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                122,
                90,
                24,
                ID_PTS_EXHARD,
                font,
            );
            label(&mut st.page_adv, 12, 160, 176, "HARD CLEAR の pt");
            st.h_pts_hard = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                160,
                90,
                24,
                ID_PTS_HARD,
                font,
            );
            label(&mut st.page_adv, 12, 198, 176, "CLEAR / EASY CLEAR の pt");
            st.h_pts_clear = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                198,
                90,
                24,
                ID_PTS_CLEAR,
                font,
            );
            label(&mut st.page_adv, 12, 236, 176, "レベルダウンする連敗数");
            st.h_fail_streak = make(
                hwnd,
                w!("EDIT"),
                "",
                WS_BORDER.0 | (ES_AUTOHSCROLL as u32) | (ES_NUMBER as u32),
                196,
                236,
                90,
                24,
                ID_FAIL_STREAK,
                font,
            );
            make(
                hwnd,
                w!("BUTTON"),
                "保存",
                BS_PUSHBUTTON as u32,
                118,
                436,
                110,
                34,
                ID_SAVE,
                font,
            );
            make(
                hwnd,
                w!("BUTTON"),
                "開始",
                BS_PUSHBUTTON as u32,
                252,
                436,
                110,
                34,
                ID_START,
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
            set_text(
                st.h_day_start,
                &cfg_i64(&st.cfg, "day_start_hour").unwrap_or(0).to_string(),
            );
            for (_, name) in HUD_ANCHORS {
                combo_add(st.h_anchor, name);
            }
            let cur_anchor =
                cfg_str(&st.cfg, "hud_anchor").unwrap_or_else(|| "custom".into());
            let a_idx = HUD_ANCHORS
                .iter()
                .position(|(k, _)| *k == cur_anchor)
                .unwrap_or(0);
            send(st.h_anchor, CB_SETCURSEL, a_idx, 0);
            set_text(
                st.h_hud_w,
                &cfg_i64(&st.cfg, "hud_width").unwrap_or(460).to_string(),
            );
            set_text(
                st.h_hud_alpha,
                &cfg_i64(&st.cfg, "hud_bg_alpha").unwrap_or(26).to_string(),
            );
            set_check(st.h_best, cfg_bool(&st.cfg, "hud_show_best", true));
            set_check(st.h_legend, cfg_bool(&st.cfg, "hud_show_legend", true));
            set_check(st.h_remain, cfg_bool(&st.cfg, "hud_show_remain", true));
            set_check(st.h_countdown, cfg_bool(&st.cfg, "hud_show_countdown", true));
            set_text(
                st.h_up_points,
                &cfg_i64(&st.cfg, "up_points").unwrap_or(3).to_string(),
            );
            set_text(
                st.h_pts_exhard,
                &cfg_i64(&st.cfg, "pts_exhard").unwrap_or(3).to_string(),
            );
            set_text(
                st.h_pts_hard,
                &cfg_i64(&st.cfg, "pts_hard").unwrap_or(2).to_string(),
            );
            set_text(
                st.h_pts_clear,
                &cfg_i64(&st.cfg, "pts_clear").unwrap_or(1).to_string(),
            );
            set_text(
                st.h_fail_streak,
                &cfg_i64(&st.cfg, "fail_streak").unwrap_or(3).to_string(),
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
            // タブのページ振り分け (コントロール本体は st の各フィールドから集める)
            st.page_main.extend([
                st.h_table,
                st.h_level,
                st.h_minutes,
                st.h_result_sec,
                st.h_mode,
                st.h_gas,
                st.h_gauge,
            ]);
            st.page_report.extend([st.h_report, st.h_day_start]);
            st.page_hud.extend([
                st.h_hud,
                st.h_anchor,
                st.h_hud_w,
                st.h_hud_alpha,
                st.h_best,
                st.h_legend,
                st.h_remain,
                st.h_countdown,
                st.h_preview,
                st.h_note,
            ]);
            st.page_adv.extend([
                st.h_up_points,
                st.h_pts_exhard,
                st.h_pts_hard,
                st.h_pts_clear,
                st.h_fail_streak,
            ]);
            show_page(st, 0);
            // 値を正規化 (クランプ等) してから、保存済みスナップショットを取る。
            // 以後の変更は「未保存」として検出できる
            collect(st);
            st.saved_cfg = st.cfg.clone();
        });
    }

    /// タブ切り替え: 選択中ページのコントロールだけを表示する
    unsafe fn show_page(st: &State, page: i32) {
        for h in &st.page_main {
            let _ = ShowWindow(*h, if page == 0 { SW_SHOW } else { SW_HIDE });
        }
        for h in &st.page_report {
            let _ = ShowWindow(*h, if page == 1 { SW_SHOW } else { SW_HIDE });
        }
        for h in &st.page_hud {
            let _ = ShowWindow(*h, if page == 2 { SW_SHOW } else { SW_HIDE });
        }
        for h in &st.page_adv {
            let _ = ShowWindow(*h, if page == 3 { SW_SHOW } else { SW_HIDE });
        }
    }

    /// 未保存の変更があるときの警告。
    /// 戻り値は MessageBox のボタン (6=はい / 7=いいえ / 2=キャンセル)
    unsafe fn warn_unsaved(hwnd: HWND) -> i32 {
        let t: Vec<u16> = "保存されていない変更があります。\n保存してから開始しますか？"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let c: Vec<u16> = "未保存の変更"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        MessageBoxW(
            hwnd,
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            MB_YESNOCANCEL | MB_ICONWARNING,
        )
        .0
    }

    /// プレビューHUD用の一時設定 (hud-preview.json) を書く。
    /// 内容が変わったときだけ書き直す
    unsafe fn write_preview(st: &mut State, force: bool) {
        collect(st);
        let mut v = st.cfg.clone();
        // 選択中の表・レベルの表示名をダミー状態用に渡す
        let t_sel = combo_sel(st.h_table);
        if t_sel >= 0 && (t_sel as usize) < st.tables.len() {
            let (tid, label) = st.tables[t_sel as usize].clone();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("_preview_table".into(), json!(label));
                let l_sel = combo_sel(st.h_level);
                if let Some(levels) = st.levels_map.get(&tid) {
                    if l_sel >= 0 && (l_sel as usize) < levels.len() {
                        obj.insert(
                            "_preview_level".into(),
                            json!(levels[l_sel as usize].clone()),
                        );
                    }
                }
            }
        }
        let text = serde_json::to_string(&v).unwrap_or_default();
        if force || text != st.preview_written {
            let _ = std::fs::write(crate::hud::preview_cfg_path(), &text);
            st.preview_written = text;
        }
    }

    /// プレビュー用にランダムな譜面を選ぶ。
    /// 選択中の表・レベルを優先し、無ければ表内→全表の順に広げる
    fn pick_random_chart(lib: &mut crate::db::Library, cfg: &Value) -> Option<String> {
        let table_id = crate::config::cfg_i64(cfg, "table_id").unwrap_or(0);
        let level = crate::config::cfg_str(cfg, "start_level").unwrap_or_default();
        let mut pool: Vec<String> = lib
            .charts_in_level(table_id, &level)
            .into_iter()
            .filter_map(|c| c.path)
            .collect();
        if pool.is_empty() {
            for lv in lib.levels(table_id) {
                pool.extend(
                    lib.charts_in_level(table_id, &lv)
                        .into_iter()
                        .filter_map(|c| c.path),
                );
                if !pool.is_empty() {
                    break;
                }
            }
        }
        if pool.is_empty() {
            for (tid, _) in lib.list_tables() {
                for lv in lib.levels(tid) {
                    pool.extend(
                        lib.charts_in_level(tid, &lv)
                            .into_iter()
                            .filter_map(|c| c.path),
                    );
                    if !pool.is_empty() {
                        break;
                    }
                }
                if !pool.is_empty() {
                    break;
                }
            }
        }
        if pool.is_empty() {
            return None;
        }
        // 乱択 (xorshift、時刻シードで十分)
        let mut x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15)
            | 1;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        Some(pool[(x % pool.len() as u64) as usize].clone())
    }

    /// プレビューの背景用に bmz-player の Viewer を起動する。
    /// -P --skip-result は one-shot Viewer (自動再生・スコア保存なし) で、
    /// 1曲終わるとプロセスが終了する
    fn launch_viewer(st: &State) -> Option<std::process::Child> {
        let dir = crate::config::find_bmz_dir(&st.cfg)?;
        let con = crate::db::open_ro(&dir.join("data").join("library.db")).ok()?;
        let mut lib = crate::db::Library::with_connection(con).ok()?;
        let chart = pick_random_chart(&mut lib, &st.cfg)?;
        std::process::Command::new(dir.join("bmz-player.exe"))
            .arg("-P")
            .arg("--skip-result")
            .arg(&chart)
            .spawn()
            .ok()
    }

    unsafe fn stop_preview(hwnd: HWND, st: &mut State) {
        let _ = KillTimer(hwnd, PREVIEW_TIMER_ID);
        if let Some(mut child) = st.preview.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(mut child) = st.bmz_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        st.preview_written.clear();
        let _ = std::fs::remove_file(crate::hud::preview_cfg_path());
        set_text(st.h_preview, "プレビュー表示");
    }

    unsafe fn toggle_preview(hwnd: HWND, st: &mut State) {
        if st.preview.is_some() {
            stop_preview(hwnd, st);
            return;
        }
        write_preview(st, true);
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(child) = std::process::Command::new(exe)
                .args(["hud", "--preview"])
                .spawn()
            {
                st.preview = Some(child);
                set_text(st.h_preview, "プレビュー終了");
                st.pos_seen = std::fs::read_to_string(crate::config::hud_pos_path())
                    .unwrap_or_default();
                SetTimer(hwnd, PREVIEW_TIMER_ID, 400, None);
                st.bmz_child = launch_viewer(st);
            }
        }
    }

    unsafe fn message_box(hwnd: HWND, text: &str, caption: &str) {
        let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let c: Vec<u16> = caption.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = MessageBoxW(
            hwnd,
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }

    /// 「参照...」: bmz-player のインストールフォルダを選び直す。
    /// 複数インストール (バージョン違い等) がある人向けの手動指定
    unsafe fn browse_bmz_dir(hwnd: HWND, st: &mut State) {
        let dlg: IFileOpenDialog = match CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) {
            Ok(d) => d,
            Err(_) => return,
        };
        let _ = dlg.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        if dlg.Show(hwnd).is_err() {
            return; // キャンセル
        }
        let Ok(item) = dlg.GetResult() else { return };
        let Ok(pwstr) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
            return;
        };
        let path = pwstr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(pwstr.as_ptr() as *const std::ffi::c_void));
        if !path.is_empty() {
            apply_bmz_dir(hwnd, st, &path);
        }
    }

    /// 選ばれたフォルダを検証し、難易度表コンボ等を作り直す
    unsafe fn apply_bmz_dir(hwnd: HWND, st: &mut State, path: &str) {
        let dir = std::path::PathBuf::from(path);
        let lib_db = dir.join("data").join("library.db");
        if !dir.join("bmz-player.exe").exists() || !lib_db.exists() {
            message_box(
                hwnd,
                "bmz-player.exe と data\\library.db が見つかりませんでした。\nbmz-player のインストールフォルダを選んでください。",
                "フォルダの確認",
            );
            return;
        }
        let mut lib = match crate::db::Library::open(&lib_db) {
            Ok(l) => l,
            Err(_) => {
                message_box(hwnd, "library.db を開けませんでした。", "フォルダの確認");
                return;
            }
        };
        let tables = lib.list_tables();
        if tables.is_empty() {
            message_box(
                hwnd,
                "そのフォルダの bmz-player には難易度表が登録されていません。",
                "フォルダの確認",
            );
            return;
        }
        st.levels_map = tables
            .iter()
            .map(|(id, _)| (*id, lib.levels(*id)))
            .collect();
        st.tables = tables;
        st.bmz_label = path.to_string();
        cfg_set(&mut st.cfg, "bmz_dir", json!(path));
        send(st.h_table, CB_RESETCONTENT, 0, 0);
        for (_, name) in &st.tables {
            combo_add(st.h_table, name);
        }
        let cur_table = cfg_i64(&st.cfg, "table_id");
        let t_idx = cur_table
            .and_then(|id| st.tables.iter().position(|(t, _)| *t == id))
            .unwrap_or(0);
        send(st.h_table, CB_SETCURSEL, t_idx, 0);
        let tid = st.tables[t_idx].0;
        let start_level = cfg_str(&st.cfg, "start_level");
        refill_levels(st, tid, start_level.as_deref());
        set_text(st.h_bmz_label, &format!("bmz-player: {}", path));
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
        let dsh: i64 = get_text(st.h_day_start).trim().parse().unwrap_or(0);
        cfg_set(&mut st.cfg, "day_start_hour", json!(dsh.clamp(0, 23)));
        let a_sel = combo_sel(st.h_anchor);
        if a_sel >= 0 && (a_sel as usize) < HUD_ANCHORS.len() {
            cfg_set(&mut st.cfg, "hud_anchor", json!(HUD_ANCHORS[a_sel as usize].0));
        }
        let hw: i64 = get_text(st.h_hud_w).trim().parse().unwrap_or(460);
        cfg_set(&mut st.cfg, "hud_width", json!(hw.clamp(240, 1200)));
        let ha: i64 = get_text(st.h_hud_alpha).trim().parse().unwrap_or(26);
        cfg_set(&mut st.cfg, "hud_bg_alpha", json!(ha.clamp(0, 255)));
        cfg_set(&mut st.cfg, "hud_show_best", json!(checked(st.h_best)));
        cfg_set(&mut st.cfg, "hud_show_legend", json!(checked(st.h_legend)));
        cfg_set(&mut st.cfg, "hud_show_remain", json!(checked(st.h_remain)));
        cfg_set(&mut st.cfg, "hud_show_countdown", json!(checked(st.h_countdown)));
        let up: i64 = get_text(st.h_up_points).trim().parse().unwrap_or(3);
        cfg_set(&mut st.cfg, "up_points", json!(up.clamp(1, 99)));
        let pe: i64 = get_text(st.h_pts_exhard).trim().parse().unwrap_or(3);
        cfg_set(&mut st.cfg, "pts_exhard", json!(pe.clamp(0, 99)));
        let ph: i64 = get_text(st.h_pts_hard).trim().parse().unwrap_or(2);
        cfg_set(&mut st.cfg, "pts_hard", json!(ph.clamp(0, 99)));
        let pc: i64 = get_text(st.h_pts_clear).trim().parse().unwrap_or(1);
        cfg_set(&mut st.cfg, "pts_clear", json!(pc.clamp(0, 99)));
        let fs: i64 = get_text(st.h_fail_streak).trim().parse().unwrap_or(3);
        cfg_set(&mut st.cfg, "fail_streak", json!(fs.clamp(1, 99)));
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
                } else if id == ID_BROWSE {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { browse_bmz_dir(hwnd, st) };
                        }
                    });
                    LRESULT(0)
                } else if id == ID_PREVIEW {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { toggle_preview(hwnd, st) };
                        }
                    });
                    LRESULT(0)
                } else if id == ID_SAVE {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe {
                                collect(st);
                                crate::config::save_config(&st.cfg);
                                st.saved_cfg = st.cfg.clone();
                            }
                        }
                    });
                    LRESULT(0)
                } else if id == ID_START {
                    let mut proceed = false;
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { collect(st) };
                            if st.cfg == st.saved_cfg {
                                st.result = Some((true, true));
                                proceed = true;
                            } else {
                                match unsafe { warn_unsaved(hwnd) } {
                                    6 => {
                                        // はい: 保存して開始
                                        crate::config::save_config(&st.cfg);
                                        st.saved_cfg = st.cfg.clone();
                                        st.result = Some((true, true));
                                        proceed = true;
                                    }
                                    7 => {
                                        // いいえ: 保存せず開始 (今回のセッション限り)
                                        st.result = Some((true, false));
                                        proceed = true;
                                    }
                                    _ => {} // キャンセル: 編集に戻る
                                }
                            }
                        }
                    });
                    if proceed {
                        let _ = DestroyWindow(hwnd);
                    }
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                // 白背景に馴染ませる (モダンな見た目に寄せる)
                let hdc = HDC(wp.0 as *mut std::ffi::c_void);
                let _ = SetBkMode(hdc, TRANSPARENT);
                LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
            }
            WM_TIMER => {
                let mut stop = false;
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        if st.preview.is_some() {
                            // HUD が Esc で閉じられたらプレビュー全体を終了する
                            let hud_gone = match st.preview.as_mut() {
                                Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                                None => false,
                            };
                            if hud_gone {
                                stop = true;
                            } else {
                                unsafe { write_preview(st, false) };
                                // HUD のドラッグで位置ファイルが変わったら、
                                // アンカーを「カスタム」に切り替える
                                let pos_text =
                                    std::fs::read_to_string(crate::config::hud_pos_path())
                                        .unwrap_or_default();
                                if pos_text != st.pos_seen {
                                    st.pos_seen = pos_text;
                                    let _ = SendMessageW(
                                        st.h_anchor,
                                        CB_SETCURSEL,
                                        WPARAM(0),
                                        LPARAM(0),
                                    );
                                }
                                // Viewer が曲を終えて終了していたら次のランダム譜面を流す
                                let ended = match st.bmz_child.as_mut() {
                                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                                    None => false,
                                };
                                if ended {
                                    st.bmz_child = launch_viewer(st);
                                }
                            }
                        }
                    }
                });
                if stop {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            unsafe { stop_preview(hwnd, st) };
                        }
                    });
                }
                LRESULT(0)
            }
            WM_NOTIFY => {
                let hdr = unsafe { &*(lp.0 as *const NMHDR) };
                if hdr.code == TCN_SELCHANGE {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            let sel =
                                unsafe { send(st.h_tab, TCM_GETCURSEL, 0, 0) }.0 as i32;
                            unsafe { show_page(st, sel) };
                        }
                    });
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        if st.result.is_none() {
                            st.result = Some((false, false));
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
    ) -> Option<(bool, bool)> {
        if tables.is_empty() {
            return None;
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
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
                h_day_start: HWND::default(),
                h_tab: HWND::default(),
                h_bmz_label: HWND::default(),
                h_anchor: HWND::default(),
                h_hud_w: HWND::default(),
                h_hud_alpha: HWND::default(),
                h_best: HWND::default(),
                h_legend: HWND::default(),
                h_remain: HWND::default(),
                h_countdown: HWND::default(),
                h_up_points: HWND::default(),
                h_pts_exhard: HWND::default(),
                h_pts_hard: HWND::default(),
                h_pts_clear: HWND::default(),
                h_fail_streak: HWND::default(),
                h_preview: HWND::default(),
                h_note: HWND::default(),
                preview: None,
                bmz_child: None,
                preview_written: String::new(),
                pos_seen: String::new(),
                page_main: Vec::new(),
                page_report: Vec::new(),
                page_hud: Vec::new(),
                page_adv: Vec::new(),
                h_mode: HWND::default(),
                h_gauge: HWND::default(),
                h_gas: HWND::default(),
                h_hud: HWND::default(),
                h_report: HWND::default(),
                result: None,
                saved_cfg: Value::Null,
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
                hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
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
                528,
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
            Some(mut st) => {
                // プレビューが残っていたら片付ける
                if let Some(mut child) = st.preview.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(crate::hud::preview_cfg_path());
                }
                if let Some(mut child) = st.bmz_child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                match st.result {
                    Some((true, persist)) => {
                        *cfg = st.cfg;
                        Some((true, persist))
                    }
                    Some((false, _)) => Some((false, false)),
                    None => None,
                }
            }
            None => None,
        }
    }
}
