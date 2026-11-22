# bmz-sparta

**BMS スパルタ練習ツール for [bmz-player](https://github.com/hyrorre/bmz-player)**

難易度表のレベルを指定すると、そのレベルの譜面を1曲ずつ自動で選曲・起動します。
結果のランプに応じてレベルが自動で上下する「スパルタ式」の練習ツールです。

*Sparta-style training launcher for bmz-player: pick a difficulty-table level,
and it launches charts one by one, moving you up or down levels by your results.*

## ルール

- 選曲は未プレイ／未クリア優先 (変更可能)
- レベルUPは点数制 (合計3点):
  - EX HARD CLEAR 以上 = 3点 (1回でUP)
  - HARD CLEAR = 2点
  - CLEAR / EASY CLEAR = 1点
  - FAIL で点数はリセット
- 3連続 FAIL でレベルDOWN
- ゲージ等のプレイ設定は bmz-player のプロファイル設定のまま使います
  (判定はランプの強さで行うため、使うゲージで難易度が変わります)
- セッション上限 (分) で自動終了。Ctrl+C でも終了できます
- プレイ中は HUD オーバーレイに現在のレベル・点数・残り時間・自己ベストを表示します

## 使い方

1. [Releases](https://github.com/koharuch/bmz-sparta/releases) から
   `bmz-sparta.exe` をダウンロードして、好きなフォルダに置く
2. `bmz-sparta.exe` を実行する
   - 設定ウィンドウが開きます。難易度表・開始レベル・セッション時間などを
     選んで「開始」を押してください
     (設定は exe と同じフォルダの `sparta.json` に保存されます)
   - bmz-player は一般的なインストール場所から自動検出します
3. あとは自動で選曲・起動が続きます
4. セッションが終わると、結果レポート (HTML) が自動で開きます。
   曲ごとの結果一覧と、X へポストするためのリンクが付いています。
   レポートは日単位で、同じ日のセッションは1つのファイルにマージされます

初回の動作確認には `--dry-run` が使えます (DB の検出と選曲だけ表示して起動しません)。

```
bmz-sparta.exe --dry-run
```

設定ウィンドウではなくコンソール対話で設定する場合は `--console`、
設定をやり直すときは `--setup` です。

## 設定 (sparta.json)

exe と同じフォルダに生成されます。手で編集することもできます。

| キー | 意味 | 既定値 |
| --- | --- | --- |
| `bmz_dir` | bmz-player.exe のあるフォルダ | 自動検出 |
| `profile` | score.db のプロファイル名 | 最初に見つかったもの |
| `table_id` | 使う難易度表の ID | 初回に対話で選択 |
| `start_level` | 開始レベル | 初回に対話で入力 |
| `minutes` | セッション時間 (分) | 60 |
| `result_seconds` | 曲と曲の間のインターバル (秒)。曲終了・スコア保存を検知してから数える | 20 |
| `day_start_hour` | 結果レポートの日付の切り替わり (時、0-23)。この時刻までは前日分として数える | 0 |
| `pick_mode` | 選曲方式: `priority` / `noplay_failed` / `random` | `priority` |
| `gauge` | 起動時のゲージ上書き。プロファイルは変更しない (設定GUIの既定: `ex-hard`) | `ex-hard` |
| `gas` | 起動時の GAS 上書き (設定GUIの既定: `best-clear`) | `best-clear` |
| `hud` | HUD を表示するか | `true` |
| `report` | 終了時に結果レポート (HTML) を生成して開くか | `true` |
| `up_points` / `pts_exhard` / `pts_hard` / `pts_clear` / `fail_streak` | レベル昇降ルールの上書き | 3 / 3 / 2 / 1 / 3 |

HUD の表示位置は `hud-pos.json` (`{"x":…, "y":…, "w":…, "h":…}`) で変更できます。

## 動作の仕組み

- bmz-player を `--skip-decide` 付きで1曲ずつ起動します
  (Decide 画面を通さず直接プレイへ。リザルト画面は表示したままにします)
- 結果は bmz-player の `score.db` を読み取り専用で参照して判定します。
  プレイヤー側のデータを書き換えることはありません
- リザルト表示中にスコア保存と IR 送信が完了してから、次の曲へ進みます

## 動作環境

- Windows 10 / 11 (x64)
- bmz-player (動作確認: v0.5.2)
  - 難易度表が登録済みで、譜面ファイルがライブラリに取り込まれていること

## 注意

- 未署名の exe のため、初回起動時に Windows SmartScreen の警告が出ることがあります
  (「詳細情報」→「実行」で起動できます)
- bmz-player の DB 形式が将来変わると動作しなくなる可能性があります。
  その場合は Issue で bmz-player のバージョンを添えて報告してください

## ビルド

```
cargo build --release
cargo test
```

## ライセンス

MIT License
