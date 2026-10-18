# 成果物の全体

この文書のパスは、リポジトリの最上位からの相対パスで記載する。

| ディレクトリ | 中身 |
|---|---|
| `optimizer/` | シミュレータと探索プログラム（Rust） |
| `save_export/` | セーブを読んで、毎ターンの記録・地図・mod の版を書き出すスクリプト（Node と bash） |
| `python_legacy/` | Python のシミュレータと診断用のスクリプト |
| `tools/` | 分析のときに使う Python のスクリプト |
| `game_mods/` | シミュレータの検証に使うゲームの mod |
| `third_party/` | 他の人が公開しているセーブ解析のプログラム |
| `data/` | セーブ、書き出した記録、ルールのデータベース、対局データ、探索の結果 |
| `docs/` | 文書 |

# シミュレータと探索プログラム

`optimizer/` は、BBG のルールで 1 ターンずつ状態を進めるシミュレータと、それを使う探索を 1 つにまとめた Rust のプログラムである。使う Rust の版は `optimizer/rust-toolchain.toml` に記載してあり、rustup がその版を自動で入れる。ビルドは次のコマンドで行い、実行ファイルは `optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt` にできる。

```
cd optimizer && cargo build --release
```

## 主なコマンド

どのコマンドも `civ6opt <コマンド> <対局データ> <オプション>` の形で実行する。

| コマンド | すること |
|---|---|
| `run` | 計画を 1 つ実行し、毎ターンの状態と点数を出力する |
| `rollout` | 各決定の候補を順に試し、点数が上がる変更を、変化がなくなるまで取り込む |
| `irollout` | 決定を 1 つ変えて後ろを詰め直すことを、時間いっぱい繰り返す |
| `decisions` | 計画の決定を、ターン・決定の種類・選んだもの・候補の一覧の形で出力する |
| `validate` | 各ターンのセーブの状態から 1 ターンだけ進め、次のセーブと比べる |

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

`save_export/` のスクリプトは、`third_party/civ6-savegame-parser` でセーブを読む。

| スクリプト | すること |
|---|---|
| `save_export/export_game.sh` | セーブのディレクトリから、プレイヤー 1 人分の毎ターンの状態を書き出す |
| `save_export/export_map.sh` | 1 つのセーブから地図を書き出す |
| `save_export/mod_versions.sh` | 対局で使った mod とその版を、セーブから読み出す |
| `save_export/players.js` | 1 つのセーブにいる全プレイヤーの産出や研究を表示する |
| `save_export/setup_parser.sh` | セーブ解析のプログラムを取得し、変更を当ててビルドする |

# 対局データ

対局データは、1 つの対局について、書き出した記録・地図・ルールのデータベースの全部の表を 1 つにまとめた JSON で、`data/games` に置く。`python_legacy/scripts/export_problem.py` が作る。`save_export/build_games.sh` を実行すると、下の表の全部の対局データを作る。

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

`python_legacy/` は、Rust に移す前に使っていた Python のシミュレータ（`civ6sim`）と、シミュレータとゲームの差を調べた診断用のスクリプトである。後で Rust に置き換えて消す。

| 場所 | すること |
|---|---|
| `python_legacy/scripts/export_problem.py` | 書き出した記録・地図・ルールのデータベースから、対局データを作る |
| `python_legacy/scripts/variant_report.py` | `run --costs` の出力を、研究や生産順の表にまとめる |
| `python_legacy/opening/` | 序盤の首都の簡略化したモデルを最適化するプログラム |

# 分析に使うスクリプト

`tools/` には、分析のときに使う Python のスクリプトを置く。`python_legacy/` のプログラムは使わず、`tools/` の中だけで動くように作る。

| 場所 | すること |
|---|---|
| `tools/ja_names.py` | ゲームの種類（`DISTRICT_ENCAMPMENT` など）と、ゲームの日本語版での名前の一覧を、`data/rules/names_ja.tsv` に出力する |

# ゲームの mod

`game_mods/Civ6EconProbe` と `game_mods/Civ6EconAutoplay` は、シミュレータの検証のために、ゲームから値を読み出したり計画を自動で打ったりする mod である。

# 設計の文書

| ファイル | 中身 |
|---|---|
| `docs/design/simulator.md` | シミュレータの設計と、確認したゲームの規則 |
| `docs/design/save_data.md` | セーブ解析のプログラムで読める項目と読めない項目 |
