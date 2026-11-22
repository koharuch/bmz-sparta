//! BMS スパルタ練習 (bmz-player 自動選曲)
//!
//! ルール:
//!   - 選んだレベルの曲を自動で1曲ずつ起動する (未プレイ優先)
//!   - レベルUPは点数制 (合計3点): EXHARD以上 = 3点 (1回でUP) /
//!     HARD = 2点 / NORMAL・EASY = 1点 (3回でUP)。fail で点数はリセット
//!   - 3連続 fail -> 前のレベルへ
//!   - ゲージ等のプレイ設定は bmz-player のプロファイル設定のまま使う
//!     (ランプは使うゲージで決まるので、UP条件はランプの強さで判定する)
//!   - セッション上限 (分) で自動終了。Ctrl+C でも終了できる
//!
//! 前提:
//!   - Windows + bmz-player がインストール済みで、難易度表が登録済みであること
//!
//! 使い方:
//!   bmz-sparta.exe            本番 (設定GUIが開く)
//!   bmz-sparta.exe --console  設定GUIを使わずコンソール対話で設定する
//!   bmz-sparta.exe --dry-run  DB検出と選曲だけ表示して起動しない (初回確認用)
//!   bmz-sparta.exe --setup    設定をやり直す
//!   bmz-sparta.exe hud        HUDオーバーレイ (本体が自動で起動する。通常は直接使わない)

mod config;
mod db;
mod hud;
mod report;
mod rules;
mod settings;
mod session;
mod status;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = if args.iter().any(|a| a == "hud") {
        hud::run_hud()
    } else {
        let dry = args.iter().any(|a| a == "--dry-run");
        let setup = args.iter().any(|a| a == "--setup");
        let console = args.iter().any(|a| a == "--console");
        session::run(dry, setup, console)
    };
    std::process::exit(code);
}
