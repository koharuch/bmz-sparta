# AGENTS.md — AIエージェント向け操作説明書

この文書は、AIエージェントが bmz-sparta をユーザーの代わりにセットアップし、
セッションを開始・運用するための説明書である。
ユーザーがこのリポジトリをAIに渡したら、この文書の手順で進めること。

## これは何か

- bmz-sparta は、BMSプレイヤー **bmz-player** (https://github.com/hyrorre/bmz-player)
  のための「スパルタ式」練習ランチャー (Windows専用、単一exe)。
- 難易度表のレベルを指定すると、そのレベルの譜面を1曲ずつ自動で選曲して
  bmz-player を起動する。結果のランプに応じてレベルが自動で上下する。
- ルールの既定値: EX HARD CLEAR以上=3pt / HARD CLEAR=2pt / CLEAR・EASY CLEAR=1pt、
  合計3ptでレベルUP、FAILでptリセット、3連敗でレベルDOWN。
  (数値は設定で変更可能)

## 前提条件 (最初に確認すること)

1. Windows 上で動いていること。
2. **bmz-player がインストール済みで、難易度表が登録済み**であること。
   bmz-sparta は bmz-player の `data\library.db` / `data\profiles\<profile>\score.db`
   を読み取り専用で開く。難易度表が無い環境では動作しないので、その場合は
   先に bmz-player 側で難易度表を追加するようユーザーに案内すること。
3. bmz-player の標準インストール先は `%LOCALAPPDATA%\Programs\BMZ Player`。
   自動検出はこの場所だけが対象。それ以外に置いてある場合は
   `--bmz-dir` か `sparta.json` の `bmz_dir` で明示する。

## セットアップ手順

1. exe を用意する。GitHub Releases から `bmz-sparta.exe` をダウンロードするか、
   Rust ツールチェインがある環境で `cargo build --release` する。
   `bmz-sparta.exe.manifest` がある場合は exe と同じフォルダに置く
   (設定画面の見た目が現代的になる。無くても動作する)。
2. 任意のフォルダに exe を置く。設定・状態ファイルは全て exe と同じフォルダに
   生成される (ポータブル形式)。
3. 環境確認をドライランで行う:

   ```
   bmz-sparta.exe run --dry-run
   ```

   bmz-player の検出、難易度表の一覧、最初の選曲が表示されればOK。
   実際には起動しない。エラーになる場合は「トラブルシューティング」参照。

## セッションの開始 (エージェント用CLI)

GUIも対話も使わず、引数だけで開始するのがエージェント向けの正規ルート。

```
bmz-sparta.exe run --table 1 --level 7 --minutes 15
```

| オプション | 意味 | 値 |
|---|---|---|
| `--bmz-dir <path>` | bmz-player のフォルダ | フルパス |
| `--table <id>` | 難易度表ID | `run --dry-run` や GUI で確認できる数値 |
| `--level <lv>` | 開始レベル | 表のレベル文字列 (例: `7`) |
| `--minutes <n>` | セッション時間 (分) | 1以上 |
| `--interval <sec>` | 曲間の待機 (秒) | 0以上 |
| `--pick <mode>` | 選曲方式 | `priority` (未プレイ優先) / `noplay_failed` / `random` |
| `--gauge <g>` | ゲージ | `assist-easy` / `easy` / `normal` / `hard` / `ex-hard` / `hazard` |
| `--gas <g>` | GAS | `off` / `continue` / `hard-to-groove` / `best-clear` / `select-to-under` |
| `--hud` / `--no-hud` | HUD表示 | |
| `--report` / `--no-report` | 終了時に結果HTMLを開くか | |
| `--dry-run` | 選曲計画だけ表示して起動しない | |

運用ルール:

- **未指定の項目は `sparta.json` の値が使われる。** ユーザーが普段GUIで
  使っている環境なら、多くの場合 `run --minutes 15` だけで足りる。
- CLI の指定はメモリ上だけに適用され、**`sparta.json` へは書き戻されない**
  (ユーザーのGUI設定を壊さないため)。
- `table_id` / `start_level` / `minutes` が引数と `sparta.json` のどちらからも
  解決できない場合、対話には落とさず **終了コード2** でエラー終了する。
  その場合は不足している項目をユーザーに確認してから再実行すること。
- ユーザーに確認すべきは基本的に「どの表・どのレベル・何分やるか」だけ。
  それ以外は既定値で進めてよい。長時間 (60分超) のセッションを開始する前は
  一言確認すること。

終了コード: `0` = 正常終了 / `1` = 実行時エラー / `2` = 引数・設定エラー。

## セッション中の状態確認

セッションの進行は exe と同じフォルダの **`sparta-status.json`** に
随時書き出される (原子的更新)。エージェントはこれを読んで報告する。

主なフィールド:

| フィールド | 意味 |
|---|---|
| `state` | `playing` (プレイ中) / `result` (リザルト・曲間) / `ended` (終了) |
| `table` / `level` | 現在の難易度表名 / レベル (シンボル付き、例: `★7`) |
| `title` | 現在の曲名 |
| `song` / `cleared` | 何曲目か / クリア曲数 |
| `points` / `up_points` | 現在のpt / レベルUPに必要なpt |
| `lose` / `lose_max` | 現在の連敗数 / レベルDOWNする連敗数 |
| `last_lamp` / `last_score_fmt` / `last_bp` | 直近の結果 (ランプ / スコア / BP) |
| `prev_lamp` / `prev_score_fmt` / `prev_bp` | 自己ベスト |
| `deadline` | セッション終了予定 (Unix秒) |
| `skip_deadline` | 次の曲が始まる時刻 (Unix秒、`state=result` のとき) |

セッションを止めるには bmz-sparta のプロセスを終了する (ユーザーに確認してから)。
セッションが終わると `state` が `ended` になり、結果レポート
`result-YYYYMMDD.html` が生成される (同じ日のセッションは1ファイルにマージされる)。

## 設定ファイル `sparta.json`

exe と同じフォルダ。GUI の設定画面もこのファイルを読み書きしている。

| キー | 意味 |
|---|---|
| `bmz_dir` | bmz-player のフォルダ (手動指定。最優先で使われ、自動検出で上書きされることは決してない) |
| `table_id` / `start_level` | 難易度表ID / 開始レベル |
| `minutes` / `result_seconds` | セッション時間 (分) / 曲間の待機 (秒) |
| `pick_mode` / `gauge` / `gas` | 選曲方式 / ゲージ / GAS (値はCLIの表と同じ) |
| `day_start_hour` | 日付の切り替わり時刻 (0-23。結果レポートの日単位) |
| `hud` / `report` | HUD表示 / レポート自動表示 |
| `hud_width` / `hud_bg_alpha` | HUDの幅 (px) / 背景の濃さ (0-255) |
| `hud_show_best` / `hud_show_legend` / `hud_show_remain` / `hud_show_countdown` | HUDの表示項目 |
| `hud_presets` | HUD位置プリセット `[{name, x, y}]` |
| `up_points` / `pts_exhard` / `pts_hard` / `pts_clear` / `fail_streak` | ルールの数値 |

注意:

- **GUIが開いている間やセッション実行中に `sparta.json` を直接編集しないこと。**
  GUIの保存で上書きされる。編集はアプリが閉じているときに行う。
- HUDの位置とサイズは別ファイル `hud-pos.json` (`{x, y, w, h}`)。
- `hud-preview.json` と `sparta-status.json` は実行時ファイルなので編集しない。

## やってはいけないこと

- bmz-player の `library.db` / `score.db` へ書き込まない (読み取り専用が設計)。
- `bmz-sparta.exe hud` サブコマンドを直接起動しない (本体が自動で起動する内部用)。
- セッション実行中の exe を差し替えない。
- ユーザーの手動指定 (`bmz_dir`) を、確認なしに別の場所へ変えない。

## トラブルシューティング

| 症状 | 対処 |
|---|---|
| 「bmz-player が見つかりません」 | 標準インストール先に無い。ユーザーに場所を聞いて `--bmz-dir` を付けるか、`sparta.json` の `bmz_dir` に書く |
| 「設定が不足しています」 (終了コード2) | 不足項目を引数で渡す。表IDが分からなければ `run --dry-run` の表示やGUIで確認 |
| 「選んだ表にレベルがありません」/ 難易度表が空 | bmz-player 側に難易度表が未登録。先に bmz-player で表を追加する |
| セッションが始まらない | `run --dry-run` で検出と選曲ができるか切り分ける。プロセスが残っている場合は前回のセッションが生きていないか確認 |
| HUDが出ない | `sparta.json` の `hud` が `true` か確認。位置が画面外の場合は `hud-pos.json` を削除すると既定位置に戻る |

## 開発

- ビルド: `cargo build --release` (Windows)。テスト: `cargo test`。
- CI: push でビルド+テスト (`.github/workflows/build.yml`)、
  `v*` タグで GitHub Release に exe を添付 (`.github/workflows/release.yml`)。
- 人間向けの使い方は `README.md` を参照。
