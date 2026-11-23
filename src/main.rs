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
//!   bmz-sparta.exe run [..]   GUI・対話なしで即開始 (AIエージェント向け。詳細は --help)
//!   bmz-sparta.exe --console  設定GUIを使わずコンソール対話で設定する
//!   bmz-sparta.exe --dry-run  DB検出と選曲だけ表示して起動しない (初回確認用)
//!   bmz-sparta.exe --setup    設定をやり直す
//!   bmz-sparta.exe hud        HUDオーバーレイ (本体が自動で起動する。通常は直接使わない)

mod cli;
mod config;
mod db;
mod hud;
mod report;
mod rules;
mod settings;
mod session;
mod status;

const USAGE: &str = "\
使い方:
  bmz-sparta.exe                 設定GUIを開いて開始
  bmz-sparta.exe run [オプション]  GUI・対話なしで即開始 (AIエージェント向け)
  bmz-sparta.exe --console       コンソール対話で設定して開始
  bmz-sparta.exe --help          この使い方を表示

run のオプション (未指定なら sparta.json の値を使う。指定はファイルに書き戻されない):
  --bmz-dir <path>    bmz-player のフォルダ
  --table <id>        難易度表ID
  --level <lv>        開始レベル (例: 7)
  --minutes <n>       セッション時間 (分)
  --interval <sec>    次の曲までの待機 (秒)
  --pick <mode>       priority | noplay_failed | random
  --gauge <g>         assist-easy | easy | normal | hard | ex-hard | hazard
  --gas <g>           off | continue | hard-to-groove | best-clear | select-to-under
  --hud / --no-hud    HUD表示の有無
  --report / --no-report  終了時に結果HTMLを開くか
  --dry-run           選曲計画だけ表示して実際には起動しない

終了コード: 0=正常 / 1=実行時エラー / 2=引数・設定エラー (対話には落とさない)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        println!("{USAGE}");
        return;
    }
    let preview = args.iter().any(|a| a == "--preview");
    let code = if args.iter().any(|a| a == "hud") {
        hud::run_hud(preview)
    } else if args.first().map(|s| s.as_str()) == Some("run") {
        match cli::parse(&args[1..]) {
            Ok(ra) => session::run(ra.dry, false, false, Some(ra.overrides)),
            Err(e) => {
                eprintln!("エラー: {e}\n\n{USAGE}");
                2
            }
        }
    } else {
        let dry = args.iter().any(|a| a == "--dry-run");
        let setup = args.iter().any(|a| a == "--setup");
        let console = args.iter().any(|a| a == "--console");
        session::run(dry, setup, console, None)
    };
    std::process::exit(code);
}
