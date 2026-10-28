# 成果物の全体

この文書のパスは、リポジトリの最上位からの相対パスで記載する。

| ディレクトリ | 中身 |
|---|---|
| `optimizer/` | シミュレータと探索プログラム（Rust） |
| `save_export/` | セーブから記録・地図・mod の版を書き出し、対局データを作るスクリプト（bash） |
| `python_legacy/` | Python のシミュレータと診断用のスクリプト |
| `tools/` | 分析のときに使う Python のスクリプト |
| `viewer/` | 対局を地図の上でターンごとに再生するブラウザのビューワ（TypeScript と Vite） |
| `game_mods/` | シミュレータの検証に使うゲームの mod |
| `data/` | セーブ、書き出した記録、ルールのデータベース、対局データ、探索の結果 |
| `docs/` | 文書 |

# シミュレータと探索プログラム

`optimizer/` は、BBG のルールで 1 ターンずつ状態を進めるシミュレータと、それを使う探索を 1 つにまとめた Rust のプログラムである。使う Rust の版は `optimizer/rust-toolchain.toml` に記載してあり、rustup がその版を自動で入れる。ビルドは次のコマンドで行い、実行ファイルは OS に関係なく `optimizer/target/release/civ6opt`（Windows では `civ6opt.exe`）にできる。

```
cd optimizer && cargo build --release
```

## 主なコマンド

`build`、`text`、`export`、`map` 以外のコマンドは、`civ6opt <コマンド> <対局データ> <オプション>` の形で実行する。

| コマンド | すること |
|---|---|
| `run` | 計画を 1 つ実行し、毎ターンの状態と点数を出力する |
| `rollout` | 各決定の候補を順に試し、点数が上がる変更を、変化がなくなるまで取り込む |
| `irollout` | 決定を 1 つ変えて後ろを詰め直すことを、時間いっぱい繰り返す |
| `decisions` | 計画の決定を、ターン・決定の種類・選んだもの・候補の一覧の形で出力する |
| `validate` | 各ターンのセーブの状態から 1 ターンだけ進め、次のセーブと比べる |
| `build` | 書き出した記録・地図・ルールのデータベースから、対局データを作る |
| `text` | ゲームの英語の文章のデータベース `data/rules/text_en.sqlite` を作る |
| `export` | セーブのディレクトリから、プレイヤー 1 人分の毎ターンの記録を書き出す |
| `map` | 1 つのセーブから地図を書き出す |

## 条件と点数の指定

| オプション | 指定するもの |
|---|---|
| `--turn T` | 点数を足すのを止めるターン |
| `--require X:N@T` | T ターンまでに X を N 個持つことを条件にする |
| `--require-weight W` | 条件を 1 つ満たさないときの罰点 |
| `--reward X:P@T` | T ターンまでに持つ X 1 個につき、P 点を加える |
| `--team` | 味方との金と贅沢資源のやり取り、同盟国を記録から取り込む |
| `--allow-military` | 内政に効かない軍事ユニットの生産を許す |
| `--freeze '[決定の一覧]'` | 指定した決定を変えない |
| `--window N --passes P --kick-to T` | `irollout` の詰め直しの範囲と回数、変える決定の範囲 |

#`--require` の X には、技術、社会制度、ユニット、区域、建物、偉人の種類を指定できる。`@T` を省くと、`--turn` のターンで判定する。

# 途中で止まった探索の再開

探索は、点数が上がるたびに計画を `--out` のファイルに保存する。途中で止まったときは、そのファイルを `--init` に指定すると、続きから回せる。

# セーブの書き出し

セーブは、`civ6opt` の `export` と `map` のコマンドが読む。セーブを読む処理（`optimizer/src/save/`）は、他の人が公開しているセーブ解析のプログラム（civ6-savegame-parser、MIT ライセンス）を Rust に移したもので、元のライセンスは `optimizer/data/save/NOTICE` にある。

| スクリプト | すること |
|---|---|
| `save_export/mod_versions.py` | 対局で使った mod とその版を、セーブから読み出す |

# 対局データ

対局データは、1 つの対局について、書き出した記録・地図・ルールのデータベースの全部の表を 1 つにまとめた JSON で、`data/games` に置く。`civ6opt build` が作る。`save_export/build_games.py` を実行すると、下の表の全部の対局データを作る。対局データの名前を続けて指定すると、その対局だけを作る。

| ファイル | 対局 |
|---|---|
| `rome_20261005.json` | 10/05 のローマ戦 |
| `rome_20261007b.json` | 10/07 のローマ戦 |
| `france_20261013.json` | 10/13 のフランスのマルチ戦 |
| `france_20261014.json` | 10/14 のフランスのマルチ戦 |
| `persia_20261017.json` | 10/17 のペルシャのマルチ戦 |
| `france_20261018.json` | 10/18 のフランスのマルチ戦 |

# データの配置

| 場所 | 中身 | リポジトリに入れるか |
|---|---|---|
| `data/saves` | ゲームのオートセーブのコピー。対局ごとにディレクトリを分ける | 入れない |
| `data/exports` | セーブから書き出した毎ターンの状態、地図、mod の版 | zstd で圧縮した `.zst` と mod の版を入れる |
| `data/rules` | ゲームのルールと文章のデータベース | 入れない |
| `data/games` | 対局データ | 入れない |
| `data/search` | 探索の実行スクリプト、結果の計画、記録 | 入れる |

# Python のプログラム

`python_legacy/` は、Rust に移す前に使っていた Python のシミュレータ（`civ6sim`）と、シミュレータとゲームの差を調べた診断用のスクリプトである。対局データを作るのには使わない。

| 場所 | すること |
|---|---|
| `python_legacy/scripts/variant_report.py` | `run --costs` の出力を、研究や生産順の表にまとめる |
| `python_legacy/opening/` | 序盤の首都の簡略化したモデルを最適化するプログラム |

# 分析に使うスクリプト

`tools/` には、分析のときに使う Python のスクリプトを置く。`python_legacy/` のプログラムは使わず、`tools/` の中だけで動くように作る。

| 場所 | すること |
|---|---|
| `tools/ja_names.py` | ゲームの種類（`DISTRICT_ENCAMPMENT` など）と、ゲームの日本語版での名前の一覧を、`data/rules/names_ja.tsv` に出力する |
| `tools/civ6opt.py` | ほかのスクリプトが読み込む部品。OS に合わせて `civ6opt` の実行ファイルを探し、優先度を下げて動かす |
| `tools/job.py` | `civ6opt` を端末から切り離して動かし、`data/jobs/<名前>` に状態と出力を残す |
| `tools/replay.py` | 記録したプレイを再現し、記録とターンごとに比べる（`check`）。再現の精度を対局ごとにまとめる（`summary`） |
| `tools/plan_report.py` | 計画の対局を、ターンごとの決定（生産、購入、労働者の行動、研究、政策、総督など）の一覧にして Markdown に出力する |
| `tools/game_icons.py` | ゲーム・DLC・mod のフォルダにあるアイコン（産出、区域、改善、資源、労働者、開拓者）を PNG にして、ビューワ（`viewer/`）が読む場所に出力する。アイコン集のファイル（`Icons.blp`）の形式は、スクリプトの冒頭に書いてある |
| `tools/viewer_data.py` | 対局の地図と、記録や計画の対局のターンごとの盤面と決定を、ビューワ（`viewer/`）が読む JSON に出力する |
| `tools/replay_check.py` | `civ6opt run --genome record` の出力と記録を、ターンごとに比べる |
| `tools/learn_train.py` | `civ6opt collect` と `irollout --rows` が書き出した選択肢の記録から、選択肢の点数への影響を学習する |
| `tools/compare_methods.py` | 探索の手法を、同じ条件・同じ時間で比べる |
| `tools/long_ma.py` | 落ちても保存した最良の計画から再開する、長い探索 |
| `optimizer/scripts/win_run.sh` | WSL から、Windows で直接 `civ6opt` を動かす。WSL を使うときだけ要る |

# ゲームの mod

`game_mods/Civ6EconProbe` と `game_mods/Civ6EconAutoplay` は、シミュレータの検証のために、ゲームから値を読み出したり計画を自動で打ったりする mod である。

# 設計の文書

| ファイル | 中身 |
|---|---|
| `docs/design/simulator.md` | シミュレータの設計と、確認したゲームの規則 |
| `docs/design/save_data.md` | セーブ解析のプログラムで読める項目と読めない項目 |
