import "./app.css";
import { Chart, type Series } from "./chart";
import { MapView, type View } from "./map";
import { loadIcons, icon } from "./icons";
import { PLAY_COLOURS, districtStyle } from "./style";
import type { City, GameData, IndexEntry, Play, TurnState } from "./types";

const METRICS: [string, string, (s: TurnState, cum: number) => number][] = [
  ["fp", "食料と生産の累計", (_, cum) => cum],
  ["food", "食料", (s) => s.output[0]],
  ["prod", "生産", (s) => s.output[1]],
  ["sci", "科学", (s) => s.output[3]],
  ["cul", "文化", (s) => s.output[4]],
  ["gold", "ゴールド", (s) => s.output[2]],
  ["pop", "人口", (s) => s.cities.reduce((a, c) => a + c.pop, 0)],
  ["cities", "都市の数", (s) => s.cities.length],
];

const YIELD_NAMES = ["食料", "生産", "ゴールド", "科学", "文化", "信仰"];

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;
const el = (tag: string, cls = "", text = "") => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
};
const fmt = (v: number) => (Math.abs(v) >= 100 ? Math.round(v) : Math.round(v * 10) / 10).toLocaleString();

interface Column {
  play: Play;
  colour: string;
  byTurn: Map<number, TurnState>;
  cum: Map<number, number>;
  map: MapView;
  info: HTMLElement;
  tab: "turn" | "cities";
}

class App {
  private data!: GameData;
  private columns: Column[] = [];
  private view: View = { scale: 10, ox: 0, oy: 0 };
  private turn = 0;
  private range: [number, number] = [0, 0];
  private timer: number | null = null;
  private selected: string | null = null;
  private hover: [number, number] | null = null;
  private chart: Chart;
  private metric = "fp";
  private frame = 0;

  constructor(index: IndexEntry[]) {
    const games = $<HTMLSelectElement>("#game");
    for (const g of index) games.append(new Option(g.name, g.name));
    games.onchange = () => this.load(games.value);
    $<HTMLSelectElement>("#left").onchange = () => this.build();
    $<HTMLSelectElement>("#right").onchange = () => this.build();
    const slider = $<HTMLInputElement>("#turn");
    slider.oninput = () => this.go(Number(slider.value));
    $("#prev").onclick = () => this.go(this.turn - 1);
    $("#next").onclick = () => this.go(this.turn + 1);
    $("#first").onclick = () => this.go(this.range[0]);
    $("#last").onclick = () => this.go(this.range[1]);
    $("#play").onclick = () => this.toggle();
    $<HTMLSelectElement>("#speed").onchange = () => this.timer !== null && (this.toggle(), this.toggle());
    $<HTMLButtonElement>("#fit").onclick = () => this.fit();
    $<HTMLInputElement>("#yields").onchange = () => this.redraw();
    const metric = $<HTMLSelectElement>("#metric");
    for (const [k, label] of METRICS) metric.append(new Option(label, k));
    metric.onchange = () => {
      this.metric = metric.value;
      this.drawChart();
    };
    this.chart = new Chart($("#chart"), (t) => this.go(t));
    window.addEventListener("keydown", (e) => {
      if ((e.target as HTMLElement).tagName === "SELECT") return;
      if (e.key === "ArrowLeft") this.go(this.turn - 1);
      else if (e.key === "ArrowRight") this.go(this.turn + 1);
      else if (e.key === " ") {
        e.preventDefault();
        this.toggle();
      } else return;
    });
    const want = new URLSearchParams(location.search).get("game");
    this.load(index.find((g) => g.name === want)?.name ?? index[index.length - 1].name);
  }

  private n(k: string | null | undefined): string {
    return k ? this.data.names[k] ?? k.replace(/^[A-Z]+_/, "") : "なし";
  }

  private async load(name: string) {
    $<HTMLSelectElement>("#game").value = name;
    this.data = await (await fetch(`data/${name}.json`)).json();
    const left = $<HTMLSelectElement>("#left"), right = $<HTMLSelectElement>("#right");
    left.innerHTML = right.innerHTML = "";
    right.append(new Option("（並べない）", "-1"));
    this.data.plays.forEach((p, i) => {
      left.append(new Option(p.label, String(i)));
      right.append(new Option(p.label, String(i)));
    });
    left.value = "0";
    right.value = this.data.plays.length > 1 ? "1" : "-1";
    this.build();
  }

  private build() {
    const host = $("#columns");
    host.innerHTML = "";
    const picks = [Number($<HTMLSelectElement>("#left").value), Number($<HTMLSelectElement>("#right").value)].filter((i) => i >= 0);
    this.columns = picks.map((pi, ci) => {
      const play = this.data.plays[pi];
      const colour = PLAY_COLOURS[ci];
      const col = el("div", "column");
      const head = el("div", "colhead");
      const swatch = el("span", "swatch");
      swatch.style.background = colour;
      head.append(swatch, el("b", "", play.label), el("span", "muted", `点数 ${fmt(play.score)}、食料と生産の累計 ${fmt(play.total)}`));
      const mapBox = el("div", "map");
      const info = el("div", "info");
      col.append(head, mapBox, info);
      host.append(col);
      const byTurn = new Map(play.turns.map((s) => [s.turn, s]));
      const cum = new Map<number, number>();
      let acc = 0;
      for (const s of play.turns) cum.set(s.turn, (acc += s.output[0] + s.output[1]));
      const map = new MapView(mapBox, this.data, this.view, colour, () => this.redraw(), (xy) => this.setHover(xy), (c) => this.select(c));
      return { play, colour, byTurn, cum, map, info, tab: "turn" as const };
    });
    const turns = this.columns.flatMap((c) => c.play.turns.map((s) => s.turn));
    this.range = [Math.min(...turns), Math.max(...turns)];
    const slider = $<HTMLInputElement>("#turn");
    slider.min = String(this.range[0]);
    slider.max = String(this.range[1]);
    this.turn = Math.min(Math.max(this.turn || this.range[1], this.range[0]), this.range[1]);
    requestAnimationFrame(() => {
      this.fit();
      this.go(this.turn);
    });
  }

  private fit() {
    if (!this.columns.length) return;
    this.columns[0].map.fit(this.columns.map((c) => c.play.turns[c.play.turns.length - 1]));
    this.redraw();
  }

  private toggle() {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
      $("#play").textContent = "▶";
      return;
    }
    if (this.turn >= this.range[1]) this.go(this.range[0]);
    $("#play").textContent = "⏸";
    this.timer = window.setInterval(() => {
      if (this.turn >= this.range[1]) this.toggle();
      else this.go(this.turn + 1);
    }, Number($<HTMLSelectElement>("#speed").value));
  }

  private go(turn: number) {
    const step = turn === this.turn + 1;
    this.turn = Math.max(this.range[0], Math.min(this.range[1], turn));
    $<HTMLInputElement>("#turn").value = String(this.turn);
    $("#turnlabel").textContent = `T${this.turn}`;
    for (const c of this.columns) {
      c.map.setTurn(c.byTurn.get(this.turn), c.byTurn.get(this.turn - 1));
      this.renderInfo(c);
    }
    this.drawChart();
    this.animate(step);
  }

  /** draws the turn's moves: over a moment after a step forward, else at once */
  private animate(step: boolean) {
    const id = ++this.frame;
    const ms = Math.min(450, Number($<HTMLSelectElement>("#speed").value) * 0.8);
    const t0 = performance.now();
    const tick = () => {
      if (id !== this.frame) return;
      const p = step ? Math.min(1, (performance.now() - t0) / ms) : 1;
      for (const c of this.columns) c.map.progress = p;
      this.redraw();
      if (p < 1) requestAnimationFrame(tick);
    };
    tick();
  }

  private pending = false;

  /** redraws once on the next frame (as icons arrive) */
  redrawSoon() {
    if (this.pending) return;
    this.pending = true;
    requestAnimationFrame(() => {
      this.pending = false;
      this.redraw();
    });
  }

  private redraw() {
    const yields = $<HTMLInputElement>("#yields").checked;
    for (const c of this.columns) {
      c.map.hover = this.hover;
      c.map.selected = this.selected;
      c.map.showYields = yields;
      c.map.draw();
    }
  }

  private setHover(xy: [number, number] | null) {
    this.hover = xy;
    this.redraw();
    const tip = $("#tip");
    if (!xy) {
      tip.hidden = true;
      return;
    }
    tip.hidden = false;
    tip.innerHTML = "";
    const p = this.columns[0].map.info(...xy)!.plot;
    tip.append(el("div", "tiphead", `(${xy[0]},${xy[1]}) ${this.n(p[2])}${p[4] ? "・" + this.n(p[4]) : ""}`));
    for (const c of this.columns) {
      const i = c.map.info(...xy);
      if (!i) continue;
      const parts = [
        i.feature && this.n(i.feature),
        i.centre && `${this.n(i.centre.name)}の都心`,
        i.district && `${this.n(i.district.type)}${i.district.done ? "" : "（建設中）"}`,
        i.improvement && this.n(i.improvement),
        i.city && !i.centre && `${this.n(i.city.name)}の土地`,
        i.worked && "働いている",
        i.yields && YIELD_NAMES.map((n, k) => (i.yields![k] ? `${n} ${fmt(i.yields![k])}` : "")).filter(Boolean).join(" "),
      ].filter(Boolean);
      const line = el("div", "", `${c.play.label}：${parts.join("、") || "なし"}`);
      line.style.color = c.colour;
      tip.append(line);
    }
  }

  private select(city: string | null) {
    this.selected = this.selected === city ? null : city;
    if (this.selected) for (const c of this.columns) c.tab = "cities";
    for (const c of this.columns) this.renderInfo(c);
    this.redraw();
  }

  private renderInfo(c: Column) {
    const s = c.byTurn.get(this.turn);
    c.info.innerHTML = "";
    const tabs = el("div", "tabs");
    for (const [k, label] of [["turn", "このターン"], ["cities", "都市"]] as const) {
      const b = el("button", c.tab === k ? "on" : "", label);
      b.onclick = () => {
        c.tab = k;
        this.renderInfo(c);
      };
      tabs.append(b);
    }
    c.info.append(tabs);
    if (!s) {
      c.info.append(el("p", "muted", "このターンの記録はありません。"));
      return;
    }
    if (c.tab === "turn") this.renderTurn(c, s);
    else this.renderCities(c, s);
  }

  private renderTurn(c: Column, s: TurnState) {
    const o = s.output;
    const stats = el("div", "stats");
    const iron = s.strategic["RESOURCE_IRON"];
    for (const [k, v] of [
      ["都市", s.cities.length], ["人口", s.cities.reduce((a, x) => a + x.pop, 0)], ["食料", o[0]], ["生産", o[1]],
      ["科学", o[3]], ["文化", o[4]], ["ゴールド", o[2]], ["所持金", s.gold], ["累計", c.cum.get(s.turn) ?? 0],
      ...(iron !== undefined ? [["鉄", iron] as [string, number]] : []),
    ] as [string, number][]) {
      const box = el("div", "stat");
      box.append(el("span", "muted", k), el("b", "", fmt(v)));
      stats.append(box);
    }
    c.info.append(stats);
    const govs = s.governors.map((g) => {
      const [type, id, titles] = g.split(":");
      const city = s.cities[Number(id) & 0xffff];
      return `${this.n(type)}（${city ? this.n(city.name) : "未任命"}、称号 ${titles}）`;
    });
    const dl = el("dl", "facts");
    for (const [k, v] of [
      ["研究", `${this.n(s.research)}（${s.techs} 個目まで終了）`],
      ["社会制度", `${this.n(s.civic)}（${s.civics} 個目まで終了）`],
      ["政体", this.n(s.government)],
      ["政策", s.policies.map((p) => this.n(p)).join("、") || "なし"],
      ["総督", govs.join("、") || "なし"],
    ]) dl.append(el("dt", "", k), el("dd", "", v));
    c.info.append(dl);
    const ds = c.play.decisions[String(s.turn)] ?? [];
    c.info.append(el("h4", "", `このターンの決定（${ds.length}）`));
    const ul = el("ul", "decisions");
    for (const d of ds) ul.append(el("li", "", d));
    if (!ds.length) ul.append(el("li", "muted", "なし"));
    c.info.append(ul);
  }

  private renderCities(c: Column, s: TurnState) {
    const table = el("table", "cities");
    const head = el("tr");
    for (const h of ["都市", "人口", "住宅", "生産中", "進み", "食料", "生産", "科学", "文化", "区域"]) head.append(el("th", "", h));
    table.append(head);
    for (const city of s.cities) {
      const tr = el("tr", city.name === this.selected ? "sel" : "");
      tr.onclick = () => this.select(city.name);
      const ds = el("td", "ds");
      for (const d of city.districts) {
        const [col, label] = districtStyle(d.type);
        const img = icon(d.type);
        const chip = el(img ? "img" : "span", "chip", img ? "" : label);
        if (img) {
          (chip as HTMLImageElement).src = img.src;
          chip.style.opacity = d.done ? "1" : "0.45";
          chip.style.border = "0";
        } else {
          chip.style.background = d.done ? col : "transparent";
          chip.style.borderColor = col;
        }
        chip.title = this.n(d.type) + (d.done ? "" : "（建設中）");
        ds.append(chip);
      }
      const item = el("td", "item");
      const img = city.item ? icon(city.item) : null;
      if (img) {
        const im = el("img", "itemicon") as HTMLImageElement;
        im.src = img.src;
        item.append(im);
      }
      item.append(document.createTextNode(city.item ? this.n(city.item) : "（次の品目を選ぶ）"));
      const progress = city.item && city.item_cost ? `${fmt(city.item_progress)}/${fmt(city.item_cost)}` : "";
      const house = el("td", "num", fmt(city.housing));
      if (city.pop >= city.housing - 1) house.classList.add("warn");
      tr.append(el("td", "", this.n(city.name)), el("td", "num", String(city.pop)), house, item, el("td", "num", progress),
        el("td", "num", fmt(city.yields[0] ?? 0)), el("td", "num", fmt(city.yields[1] ?? 0)),
        el("td", "num", fmt(city.yields[3] ?? 0)), el("td", "num", fmt(city.yields[4] ?? 0)), ds);
      table.append(tr);
    }
    c.info.append(table);
    const city = s.cities.find((x) => x.name === this.selected);
    if (city) c.info.append(this.cityDetail(city));
  }

  private cityDetail(city: City): HTMLElement {
    const box = el("div", "detail");
    box.append(el("h4", "", this.n(city.name)));
    const dl = el("dl", "facts");
    dl.append(el("dt", "", "建物"), el("dd", "", city.buildings.map((b) => this.n(b)).join("、") || "なし"));
    dl.append(el("dt", "", "区域"), el("dd", "", city.districts.map((d) => this.n(d.type) + (d.done ? "" : "（建設中）")).join("、") || "なし"));
    dl.append(el("dt", "", "働くタイル"), el("dd", "", `${city.worked.length} 枚（所有 ${city.tiles.length} 枚）`));
    box.append(dl);
    return box;
  }

  private drawChart() {
    const f = METRICS.find((m) => m[0] === this.metric)![2];
    const series: Series[] = this.columns.map((c) => ({
      label: c.play.label,
      colour: c.colour,
      points: c.play.turns.map((s) => [s.turn, f(s, c.cum.get(s.turn) ?? 0)] as [number, number]),
    }));
    this.chart.set(series, this.range, this.turn);
    const legend = $("#legend");
    legend.innerHTML = "";
    for (const c of this.columns) {
      const s = c.byTurn.get(this.turn);
      const v = s ? fmt(f(s, c.cum.get(s.turn) ?? 0)) : "-";
      const item = el("span", "", `${c.play.label} ${v}`);
      item.style.color = c.colour;
      legend.append(item);
    }
  }
}

let app: App | null = null;
Promise.all([fetch("data/index.json").then((r) => r.json()), loadIcons(() => app?.redrawSoon())])
  .then(([index]) => (app = new App(index as IndexEntry[])))
  .catch(() => {
    document.body.innerHTML = "<p style='padding:2em'>viewer/public/data/index.json がありません。tools/viewer_data.py でデータを書き出してください。</p>";
  });
