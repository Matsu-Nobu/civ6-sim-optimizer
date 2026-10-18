# 成果物の全体

この文書のパスは、リポジトリの最上位からの相対パスで記載する。

| ディレクトリ | 中身 |
|---|---|
| `optimizer/` | シミュレータと探索プログラム（Rust） |
| `save_export/` | セーブを読んで、毎ターンの記録・地図・mod の版を書き出すスクリプト（Node と bash） |
| `python_legacy/` | Python のシミュレータと診断用のスクリプト |
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
| `run` | 計画を 1 つ実行し、毎ターンの状態と点数を出力する。`--costs` を付けると、各都市が作っている品目のコストも出力する |
| `rollout` | 各決定の候補を順に試し、点数が上がる変更を、変化がなくなるまで取り込む |
| `irollout` | 決定を 1 つランダムに変え、後ろの決定を `rollout` と同じ方法で詰め直し、点数が上がれば採用する。これを時間いっぱい繰り返す |
| `decisions` | 計画の決定を、ターン・決定の種類・選んだもの・候補の一覧の形で出力する |
| `validate` | 各ターンのセーブの状態から 1 ターンだけ進め、次のセーブと比べる |

## 条件と点数の指定

| オプション | 指定するもの |
|---|---|
| `--turn T` | 点数を足すのを止めるターン |
| `--require X:N@T` | T ターンまでに X を N 個持つことを条件にする。`@T` を省くと `--turn` のターンで判定する。X には技術、社会制度、ユニット、区域、建物、偉人の種類（`GREAT_PERSON_CLASS_GENERAL>ERA_RENAISSANCE` のように時代を絞れる）を指定できる |
| `--require-weight W` | 条件を 1 つ満たさないときの罰点 |
| `--reward X:P@T` | T ターンまでに持つ X 1 個につき、P 点を加える |
| `--team` | 味方に渡した金を計画からも差し引き、味方からもらった贅沢資源を記録から推定して加える。記録の交易路の行き先から同盟国も推定する |
| `--allow-military` | 内政に効かない軍事ユニットの生産を許す |
| `--freeze '[決定の一覧]'` | 指定した決定を変えない |
| `--window N --passes P --kick-to T` | `irollout` で、変えた決定から N ターン分を最大 P 周詰め直し、T ターンまでの決定だけを変える |

## 途中で止まった探索の再開

探索は、点数が上がるたびに計画を `--out` のファイルに保存する。途中で止まったときは、そのファイルを `--init` に指定すると、続きから回せる。

# セーブの書き出し

`save_export/` のスクリプトは、`third_party/civ6-savegame-parser` でセーブを読む。

| スクリプト | すること |
|---|---|
| `save_export/export_game.sh` | セーブのディレクトリから、プレイヤー 1 人分の毎ターンの状態を書き出す |
| `save_export/export_map.sh` | 1 つのセーブから地図を書き出す |
| `save_export/mod_versions.sh` | 対局で使った mod とその版を、セーブから読み出す |
| `save_export/players.js` | 1 つのセーブにいる全プレイヤーの産出や研究を表示する |
| `save_export/setup_parser.sh` | `third_party/civ6-savegame-parser` を取得し、`third_party/civ6-savegame-parser.patch` の変更を当てて、npm でビルドする |

# 対局データ

対局データは、1 つの対局について、書き出した記録・地図・ルールのデータベースの全部の表を 1 つにまとめた JSON で、`data/games` に置く。`python_legacy/scripts/export_problem.py` が作る。`save_export/build_games.sh` を実行すると、下の表の全部の対局データを作る。

| ファイル | 対局 |
|---|---|
| `rome_20261005.json` | 10/05 のローマ戦 |
| `rome_20261007b.json` | 10/07 のローマ戦 |
| `france_20261013.json` | 10/13 のフランスのマルチ戦 |
| `france_20261014.json` | 10/14 のフランスのマルチ戦 |
| `persia_20261017.json` | 10/17 のペルシャのマルチ戦 |

# データの配置

| 場所 | 中身 | リポジトリに入れるか |
|---|---|---|
| `data/saves` | ゲームのオートセーブのコピー。対局ごとにディレクトリを分ける | 入れない |
| `data/exports` | セーブから書き出した毎ターンの状態（`game_<日付>_p<プレイヤー番号>.jsonl`）、地図、mod の版 | zstd で圧縮した `.zst` と mod の版を入れる |
| `data/rules` | ゲームが書き出したルールのデータベースのコピー（`<対局データの名前>.sqlite`）と、ゲームの文章のデータベース（`text_en.sqlite`） | 入れない |
| `data/games` | 対局データ | 入れない |
| `data/search` | 探索の実行スクリプト、結果の計画、記録。目的ごとにディレクトリを分ける（騎乗兵は `nest`、皇帝近衛は `ms67`、ペルシャ戦は `persia`） | 入れる |

# Python のプログラム

`python_legacy/` は、Rust に移す前に使っていた Python のシミュレータ（`civ6sim`）と、シミュレータとゲームの差を調べた診断用のスクリプトである。後で Rust に置き換えて消す。

| 場所 | すること |
|---|---|
| `python_legacy/scripts/export_problem.py` | 書き出した記録・地図・ルールのデータベースから、対局データを作る |
| `python_legacy/scripts/variant_report.py` | `run --costs` の出力から、研究の完了、都市ごとの生産順、改善、食料と生産の累計をまとめて表示する |
| `python_legacy/opening/` | 序盤の首都の簡略化したモデルを最適化するプログラム |

# ゲームの mod

`game_mods/Civ6EconProbe` と `game_mods/Civ6EconAutoplay` は、シミュレータの検証のために、ゲームから値を読み出したり計画を自動で打ったりする mod である。

# 設計の文書

| ファイル | 中身 |
|---|---|
| `docs/design/simulator.md` | シミュレータの設計と、確認したゲームの規則 |
| `docs/design/save_data.md` | セーブ解析のプログラムで読める項目と読めない項目 |
