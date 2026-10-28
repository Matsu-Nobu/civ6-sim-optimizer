# 概要

Civilization VI に BBG（対戦向けにバランスを調整する mod）を入れたマルチプレイで、強い内政の進め方を計算で見つけ、自分の対局を振り返るためのプロジェクトである。自分の対局の記録をシミュレータで再現し、探索プログラムで最良の計画を作り、自分のプレイと比べる。

# 目的

## 強い内政の進め方を見つける

BBG は更新のたびに強い進め方が変わるので、勘や定石に頼らず、ルールの表から計算し直せる方法で見つけたい。作戦ごとに、必要な技術やユニットを期限までにそろえたうえで、食料と生産が最も伸びる計画を探す。

## 自分の対局を振り返る

最終的には、対局ごとに、決定ごとの損失の一覧を作りたい。決定とは、何を生産するか、どこに都市を建てるか、何を研究するか、総督をどう使うか、といった選択である。損失は、その決定を最良の計画の決定に替えたときに、食料と生産の合計がどれだけ増えるかで表す。

この一覧はまだ作っていない。今は、作戦ごとの最良の計画と自分のプレイの違いを、レポートにまとめている。

# 現状の制約

次のものは、今のシミュレータでは扱えないか、簡略化している。どれも、必要になれば対応する。

| 制約 | 今の扱い |
|---|---|
| 都市の占領 | 計算できないので、計画の中では占領が起きない前提にしている |
| 時代スコア、黄金時代と暗黒時代、献身 | 計算せず、対局の記録の時代をそのまま与えている |
| 政策の有料の入れ替え | 社会制度の完成と政体の変更のときの、無料の入れ替えだけを扱う |
| 研究の同盟の 2 段目以降 | 1 段目の、同盟国への交易路の科学だけを扱う |

# スコープ外

戦闘の勝敗と軍事の強さは計算しない。軍事が要る作戦では、必要なユニットの数を条件として与え、そのユニットを期限までにそろえるための内政だけを最適化する。

他のプレイヤーの行動も予測しない。他のプレイヤーが建てた遺産や取った偉人、ゲームの時代の変わり目は、対局の記録で起きたとおりに与える。

# 実験の再現

ゲームのルールのデータベースは著作物なのでリポジトリに入れていない。自分のゲームから作る。

## 必要なもの

Windows・macOS・Linux のどれでも同じ手順で動く。

- Civilization VI（Steam 版）
- Steam ワークショップの mod の [Better Balanced Game 7.5.1](https://steamcommunity.com/sharedfiles/filedetails/?id=2865001760)、[BBG Expanded 2.1](https://steamcommunity.com/sharedfiles/filedetails/?id=3533091092)、[Better Balanced Map 1.39.5](https://steamcommunity.com/sharedfiles/filedetails/?id=3179425402)、[Multiplayer Helper 1.8.0](https://steamcommunity.com/sharedfiles/filedetails/?id=3307282026)
- `rustup`。Windows では、加えて C のコンパイラ（Visual Studio の Build Tools か MinGW）が要る。Windows でだけ使うメモリ確保のライブラリ（mimalloc）をビルドするためである。
- Python 3.14 以降。zstd の展開に、3.14 で入った標準ライブラリの `compression.zstd` を使う。

## 手順

1. 上の 4 つの mod を入れて、記録した対局と同じ勝利条件でゲームを始める。
2. ゲームを始めると書き出される `Cache/DebugGameplay.sqlite` を、`data/rules/france_20261014.sqlite` のように対局データの名前でコピーする。
3. `optimizer/` で `cargo build --release` を実行する。実行ファイルは `optimizer/target/release/civ6opt`（Windows では `civ6opt.exe`）にできる。
4. `optimizer/target/release/civ6opt text data/rules/text_en.sqlite` を実行する。Steam を既定の場所に入れていない場合は、`--game` と `--workshop` でゲームとワークショップのフォルダを指定する。
5. `python save_export/build_games.py` を実行する。
6. `data/search/<実験>/run.sh` に記録した `civ6opt` のコマンドを実行する。長い探索は `python tools/job.py start <名前> <コマンド>` で、端末から切り離して動かす。

学習のスクリプト（`tools/learn_train.py`）を使うときは、仮想環境を作って `pip install -r requirements.txt` を実行する。

## 対局を再生して見る

`viewer/` は、対局を地図の上でターンごとに再生するブラウザのビューワである。2 つの対局（自分のプレイと探索の計画など）を左右に並べ、地図、そのターンの決定、産出、都市の一覧、ターンごとのグラフを比べられる。使うには Node.js 20 以降が要る。

1. `python tools/viewer_data.py rome21 data/games/rome_20261021.json あなた=record 探索=<計画の .json> -- <探索と同じ civ6opt の条件>` を実行して、対局のデータを `viewer/public/data/` に書き出す。`record` は記録どおりの再現を表す。
2. `python tools/game_icons.py` を実行して、地図に出す産出のアイコンを、自分のゲームのフォルダから `viewer/public/data/icons/` に書き出す。アイコンはゲームの著作物なので、リポジトリには入れていない。Pillow が要る。
3. `viewer/` で `npm install` と `npm run dev` を実行し、表示された URL をブラウザで開く。

地図には、拡大すると、都市の土地の各タイルで市民が働いたときの産出を、ゲームのアイコンで出す。労働者は前のターンの位置から動いた跡を、移動中の開拓者は出発した都市から行き先までの破線を描く。地図はマウスのホイールで拡大・縮小し、ドラッグで動かす。左右の地図は一緒に動く。左右の矢印のキーでターンを送り、スペースのキーで再生を始める。都市をクリックすると、その都市の土地が明るくなり、都市の一覧にその都市の建物と区域が出る。

# 文書の案内

初めて読むときは、この文書、`docs/approach.md`、`docs/reports/` のレポートの順に読むと、全体がつかめる。特定の日の作業、決めたことの理由、ファイルの場所を調べるときは、それぞれ `docs/history.md`、`docs/decisions.md`、`docs/artifacts.md` を読む。

| 文書 | 内容 |
|---|---|
| [approach.md](docs/approach.md) | 対局の記録を取り込んでから計画を比べるまでの進め方と、計画の点数の付け方 |
| [history.md](docs/history.md) | 日付ごとの作業と結果 |
| [decisions.md](docs/decisions.md) | 日付ごとに決めたことと、その理由 |
| [artifacts.md](docs/artifacts.md) | シミュレータ、探索プログラム、データ、スクリプト、mod の一覧と使い方 |
| [reports/search_methods.md](docs/reports/search_methods.md) | 探索の手法を比べた結果 |
| [reports/opening.md](docs/reports/opening.md) | 序盤のタイル選び・斥候・開拓者の定石 |
| [reports/france_courser_spike.md](docs/reports/france_courser_spike.md) | フランスでコーサーに上げて攻める作戦の内政の計画 |
| [reports/persia_20261017.md](docs/reports/persia_20261017.md) | 10/17 のペルシャのマルチ戦の分析 |
| [reports/gran_colombia_20261019.md](docs/reports/gran_colombia_20261019.md) | 10/19 の大コロンビアのマルチ戦の分析と、祖廟と謁見の間の比較 |
| [reports/france_20261018.md](docs/reports/france_20261018.md) | 10/18 のフランスのマルチ戦の分析 |
