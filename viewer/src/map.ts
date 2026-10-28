import { across, centre, corners, plotAt } from "./hex";
import { icon } from "./icons";
import { FEATURE, IMPROVEMENT, districtStyle, isHills, terrainColour } from "./style";
import type { City, District, GameData, Plot, TurnState } from "./types";

/** where the maps look: screen pixels per map unit and the screen point of map (0, 0); shared by every map */
export interface View {
  scale: number;
  ox: number;
  oy: number;
}

export interface TileInfo {
  plot: Plot;
  city: City | null;
  feature: string | null;
  improvement: string | null;
  district: District | null;
  worked: boolean;
  centre: City | null;
  yields: number[] | null;
}

const RESOURCE_COLOURS: Record<string, string> = { bonus: "#e8f0d8", luxury: "#e070e0", strategic: "#ff9a3c" };

/** food, production, gold, science, culture, faith: the colours drawn when the game's yield icons are missing */
const YIELD_COLOURS = ["#7cc644", "#d08a3a", "#e8c53a", "#4fb0e8", "#b05ad8", "#c8d0dc"];

/** where n icons of a tile's yields go, in icon sizes from the tile's centre */
const YIELD_LAYOUT: [number, number][][] = [
  [],
  [[0, 0]],
  [[-0.5, 0], [0.5, 0]],
  [[-1, 0], [0, 0], [1, 0]],
  [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]],
  [[-1, -0.5], [0, -0.5], [1, -0.5], [-0.5, 0.5], [0.5, 0.5]],
  [[-1, -0.5], [0, -0.5], [1, -0.5], [-1, 0.5], [0, 0.5], [1, 0.5]],
];

/** the game's yield icons (tools/game_icons.py): yield i in the column centred at x = 32 + 128 i, n of it (1 to 4)
 * in the row centred at y = 64 + 128 (n - 1), the large icon for five or more in the fifth row */
const yieldAtlas = new Image();
yieldAtlas.src = "data/icons/yields.png";

/** a settler on its way: [city it left, target x, target y, turn it founds the city] */
type SettlerTrip = [string, number, number, number];
export const settlerKey = (s: SettlerTrip) => `${s[0]}|${s[1]},${s[2]}`;

const key = (x: number, y: number) => `${x},${y}`;

/** what one play has on the map in one turn, by plot */
class TurnIndex {
  tiles = new Map<string, { city: City; feature: string | null; improvement: string | null; yields: number[] | null }>();
  districts = new Map<string, District>();
  worked = new Set<string>();
  centres = new Map<string, City>();

  constructor(state: TurnState | undefined) {
    if (!state) return;
    for (const c of state.cities) {
      for (const [x, y, f, i, ys] of c.tiles) this.tiles.set(key(x, y), { city: c, feature: f, improvement: i, yields: ys });
      for (const d of c.districts) this.districts.set(key(d.x, d.y), d);
      for (const [x, y] of c.worked) this.worked.add(key(x, y));
      this.centres.set(key(c.xy[0], c.xy[1]), c);
    }
  }

  signature(k: string): string {
    const t = this.tiles.get(k);
    const d = this.districts.get(k);
    return `${t?.city.name}|${t?.feature}|${t?.improvement}|${d?.type}|${d?.done}|${this.centres.has(k)}`;
  }
}

export class MapView {
  readonly canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private plots = new Map<string, Plot>();
  private now = new TurnIndex(undefined);
  private changed = new Set<string>();
  private state: TurnState | undefined;
  private prev: TurnState | undefined;
  hover: [number, number] | null = null;
  selected: string | null = null;
  showYields = true;
  /** how far the turn's moves are drawn (0: where units were last turn, 1: where they are now) */
  progress = 1;
  /** settler trip -> the first turn the play has it */
  settlerStarts = new Map<string, number>();

  constructor(
    parent: HTMLElement,
    private data: GameData,
    private view: View,
    private colour: string,
    private redrawAll: () => void,
    private onHover: (xy: [number, number] | null) => void,
    private onSelect: (city: string | null) => void,
  ) {
    this.canvas = document.createElement("canvas");
    parent.appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d")!;
    for (const p of data.plots) this.plots.set(key(p[0], p[1]), p);
    this.listen();
    new ResizeObserver(() => this.resize()).observe(parent);
  }

  setTurn(state: TurnState | undefined, prev: TurnState | undefined) {
    this.state = state;
    this.prev = prev;
    this.now = new TurnIndex(state);
    const before = new TurnIndex(prev);
    this.changed.clear();
    if (prev) {
      for (const k of new Set([...this.now.tiles.keys(), ...this.now.districts.keys()])) {
        if (this.now.signature(k) !== before.signature(k)) this.changed.add(k);
      }
    }
  }

  info(x: number, y: number): TileInfo | null {
    const k = key(x, y);
    const plot = this.plots.get(k);
    if (!plot) return null;
    const t = this.now.tiles.get(k);
    return {
      plot,
      city: t?.city ?? null,
      feature: t ? t.feature : plot[3],
      improvement: t?.improvement ?? null,
      yields: t?.yields ?? null,
      district: this.now.districts.get(k) ?? null,
      worked: this.now.worked.has(k),
      centre: this.now.centres.get(k) ?? null,
    };
  }

  /** the map centred on the play's cities (or the map's middle) */
  fit(states: (TurnState | undefined)[]) {
    const pts = states.flatMap((s) => s?.cities.map((c) => centre(c.xy[0], c.xy[1], this.data.height)) ?? []);
    const w = this.canvas.clientWidth || 600, h = this.canvas.clientHeight || 400;
    if (!pts.length) {
      pts.push([0, 0], centre(this.data.width - 1, 0, this.data.height));
    }
    const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
    const pad = 3;
    const [x0, x1, y0, y1] = [Math.min(...xs) - pad, Math.max(...xs) + pad, Math.min(...ys) - pad, Math.max(...ys) + pad];
    this.view.scale = Math.min(w / (x1 - x0), h / (y1 - y0));
    this.view.ox = w / 2 - ((x0 + x1) / 2) * this.view.scale;
    this.view.oy = h / 2 - ((y0 + y1) / 2) * this.view.scale;
  }

  private resize() {
    const dpr = window.devicePixelRatio || 1;
    const { clientWidth: w, clientHeight: h } = this.canvas;
    this.canvas.width = Math.round(w * dpr);
    this.canvas.height = Math.round(h * dpr);
    this.draw();
  }

  private toMap(sx: number, sy: number): [number, number] {
    return [(sx - this.view.ox) / this.view.scale, (sy - this.view.oy) / this.view.scale];
  }

  private listen() {
    const c = this.canvas;
    let drag: { x: number; y: number; ox: number; oy: number; moved: boolean } | null = null;
    c.addEventListener("wheel", (e) => {
      e.preventDefault();
      const f = Math.exp(-e.deltaY * 0.0015);
      const s = Math.max(1, Math.min(120, this.view.scale * f));
      const [mx, my] = this.toMap(e.offsetX, e.offsetY);
      this.view.scale = s;
      this.view.ox = e.offsetX - mx * s;
      this.view.oy = e.offsetY - my * s;
      this.redrawAll();
    }, { passive: false });
    c.addEventListener("pointerdown", (e) => {
      drag = { x: e.clientX, y: e.clientY, ox: this.view.ox, oy: this.view.oy, moved: false };
      c.setPointerCapture(e.pointerId);
    });
    c.addEventListener("pointermove", (e) => {
      if (drag) {
        const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
        if (drag.moved) {
          this.view.ox = drag.ox + dx;
          this.view.oy = drag.oy + dy;
          this.redrawAll();
          return;
        }
      }
      const [mx, my] = this.toMap(e.offsetX, e.offsetY);
      this.onHover(plotAt(mx, my, this.data.width, this.data.height));
    });
    c.addEventListener("pointerup", (e) => {
      if (drag && !drag.moved) {
        const [mx, my] = this.toMap(e.offsetX, e.offsetY);
        const xy = plotAt(mx, my, this.data.width, this.data.height);
        const info = xy && this.info(...xy);
        this.onSelect(info?.centre?.name ?? info?.city?.name ?? null);
      }
      drag = null;
    });
    c.addEventListener("pointerleave", () => this.onHover(null));
  }

  draw() {
    const ctx = this.ctx;
    const dpr = window.devicePixelRatio || 1;
    const { scale, ox, oy } = this.view;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = "#14181d";
    ctx.fillRect(0, 0, this.canvas.clientWidth, this.canvas.clientHeight);
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight;
    const S = (mx: number, my: number): [number, number] => [ox + mx * scale, oy + my * scale];
    const visible: [Plot, number, number][] = [];
    for (const p of this.data.plots) {
      const [cx, cy] = S(...centre(p[0], p[1], this.data.height));
      if (cx < -scale * 2 || cy < -scale * 2 || cx > w + scale * 2 || cy > h + scale * 2) continue;
      visible.push([p, cx, cy]);
    }
    const r = scale;
    const detail = scale >= 9;
    const hexPath = (cx: number, cy: number, rr: number) => {
      const cs = corners(cx, cy, rr);
      ctx.beginPath();
      ctx.moveTo(...cs[0]);
      for (let i = 1; i < 6; i++) ctx.lineTo(...cs[i]);
      ctx.closePath();
    };

    // terrain, hills and mountains, features
    for (const [p, cx, cy] of visible) {
      const k = key(p[0], p[1]);
      hexPath(cx, cy, r * 1.01);
      ctx.fillStyle = terrainColour(p[2]);
      ctx.fill();
      if (p[2].endsWith("_MOUNTAIN")) {
        ctx.fillStyle = "#d8d4ce";
        ctx.beginPath();
        ctx.moveTo(cx, cy - r * 0.55);
        ctx.lineTo(cx + r * 0.5, cy + r * 0.35);
        ctx.lineTo(cx - r * 0.5, cy + r * 0.35);
        ctx.fill();
      } else if (isHills(p[2]) && detail) {
        ctx.strokeStyle = "rgba(0,0,0,0.35)";
        ctx.lineWidth = Math.max(1, r * 0.06);
        ctx.beginPath();
        ctx.moveTo(cx - r * 0.55, cy + r * 0.45);
        ctx.quadraticCurveTo(cx - r * 0.3, cy + r * 0.1, cx - r * 0.05, cy + r * 0.45);
        ctx.stroke();
      }
      const t = this.now.tiles.get(k);
      const feature = t ? t.feature : p[3];
      if (feature) {
        const [col, label] = FEATURE[feature] ?? ["#b08040", "名"];
        if (feature === "FEATURE_FOREST" || feature === "FEATURE_JUNGLE") {
          ctx.fillStyle = col;
          for (const [dx, dy] of [[-0.35, 0.05], [0.3, -0.1], [0, 0.4]]) {
            ctx.beginPath();
            ctx.moveTo(cx + dx * r, cy + (dy - 0.32) * r);
            ctx.lineTo(cx + (dx + 0.2) * r, cy + (dy + 0.08) * r);
            ctx.lineTo(cx + (dx - 0.2) * r, cy + (dy + 0.08) * r);
            ctx.fill();
          }
        } else if (detail) {
          this.text(label, cx, cy + r * 0.05, r * 0.5, col);
        }
      }
    }

    // rivers
    ctx.strokeStyle = "#5fb4f0";
    ctx.lineWidth = Math.max(1.5, r * 0.14);
    ctx.lineCap = "round";
    for (const [p, cx, cy] of visible) {
      if (!p[5]) continue;
      const cs = corners(cx, cy, r);
      for (let e = 0; e < 6; e++) {
        if (!((p[5] >> e) & 1)) continue;
        ctx.beginPath();
        ctx.moveTo(...cs[e]);
        ctx.lineTo(...cs[(e + 1) % 6]);
        ctx.stroke();
      }
    }

    // territory, districts, city centres, improvements, resources, worked plots
    for (const [p, cx, cy] of visible) {
      const k = key(p[0], p[1]);
      const t = this.now.tiles.get(k);
      if (t) {
        hexPath(cx, cy, r);
        ctx.fillStyle = this.selected === t.city.name ? "rgba(255,255,255,0.22)" : hexA(this.colour, 0.2);
        ctx.fill();
      }
      const d = this.now.districts.get(k);
      const dIcon = d && icon(d.type);
      if (d && dIcon) {
        // the game's district icon is a hex; one being built is faint, with a dashed outline
        const size = r * 1.75;
        ctx.globalAlpha = d.done ? 1 : 0.45;
        ctx.drawImage(dIcon, cx - size / 2, cy - size / 2, size, size);
        ctx.globalAlpha = 1;
        if (!d.done) {
          hexPath(cx, cy, r * 0.8);
          ctx.setLineDash([r * 0.15, r * 0.12]);
          ctx.strokeStyle = "#fff";
          ctx.lineWidth = Math.max(1, r * 0.06);
          ctx.stroke();
          ctx.setLineDash([]);
        }
      } else if (d) {
        const [col, label] = districtStyle(d.type);
        hexPath(cx, cy, r * 0.78);
        ctx.fillStyle = d.done ? col : hexA(col, 0.35);
        ctx.fill();
        if (!d.done) {
          ctx.setLineDash([r * 0.15, r * 0.12]);
          ctx.strokeStyle = col;
          ctx.lineWidth = Math.max(1, r * 0.08);
          ctx.stroke();
          ctx.setLineDash([]);
        }
        if (scale >= 5) this.text(label, cx, cy, r * 0.75, "#fff", true);
      }
      const city = this.now.centres.get(k);
      if (city) {
        hexPath(cx, cy, r * 0.82);
        ctx.fillStyle = "#2a2f38";
        ctx.fill();
        ctx.strokeStyle = this.colour;
        ctx.lineWidth = Math.max(1.5, r * 0.14);
        ctx.stroke();
        if (scale >= 4) this.text(String(city.pop), cx, cy, r * 0.85, "#fff", true);
      }
      if (t?.improvement && !d && !city && detail) {
        // above the yield icons when they are drawn, else below the centre
        const iy = this.showYields && scale >= 14 ? cy - r * 0.6 : cy + r * 0.45;
        ctx.fillStyle = "rgba(20,20,20,0.7)";
        ctx.beginPath();
        ctx.arc(cx, iy, r * 0.27, 0, Math.PI * 2);
        ctx.fill();
        const img = icon(t.improvement);
        if (img) ctx.drawImage(img, cx - r * 0.24, iy - r * 0.24, r * 0.48, r * 0.48);
        else this.text(IMPROVEMENT[t.improvement] ?? (this.data.names[t.improvement] ?? "改")[0], cx, iy + 1, r * 0.3, "#ffe9a8");
      }
      const rIcon = p[4] && detail && !d && !city ? icon(p[4]) : null;
      if (rIcon) {
        ctx.drawImage(rIcon, cx + r * 0.12, cy - r * 0.72, r * 0.6, r * 0.6);
      } else if (p[4] && detail && !d && !city) {
        const cls = this.data.resource_classes?.[p[4]] ?? "bonus";
        ctx.fillStyle = RESOURCE_COLOURS[cls] ?? "#fff";
        ctx.beginPath();
        ctx.arc(cx + r * 0.42, cy - r * 0.38, r * 0.17, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = "rgba(0,0,0,0.6)";
        ctx.lineWidth = 1;
        ctx.stroke();
      }
      if (this.now.worked.has(k) && !city && detail) {
        ctx.fillStyle = "#fff";
        ctx.strokeStyle = "#000";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.arc(cx - r * 0.42, cy - r * 0.38, r * 0.14, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
      }
    }

    // borders: thick between the player's land and the rest, thin between its cities
    for (const [p, cx, cy] of visible) {
      const t = this.now.tiles.get(key(p[0], p[1]));
      if (!t) continue;
      const cs = corners(cx, cy, r * 0.97);
      for (let e = 0; e < 6; e++) {
        const [nx, ny] = across(p[0], p[1], e);
        const n = this.now.tiles.get(key(nx, ny));
        if (n && n.city === t.city) continue;
        ctx.strokeStyle = n ? hexA(this.colour, 0.45) : this.colour;
        ctx.lineWidth = n ? Math.max(1, r * 0.05) : Math.max(1.5, r * 0.13);
        ctx.beginPath();
        ctx.moveTo(...cs[e]);
        ctx.lineTo(...cs[(e + 1) % 6]);
        ctx.stroke();
      }
    }

    // the yields a citizen working each owned plot gets
    if (this.showYields && scale >= 14) {
      const size = r * 0.46;
      for (const [p, cx, cy] of visible) {
        const t = this.now.tiles.get(key(p[0], p[1]));
        // the city centre shows the city's population instead (its yields are in the tooltip)
        if (!t?.yields || this.now.centres.has(key(p[0], p[1]))) continue;
        const ys = t.yields.map((v, i) => [i, Math.round(v)] as [number, number]).filter(([, v]) => v > 0);
        const layout = YIELD_LAYOUT[ys.length] ?? [];
        ys.forEach(([i, v], j) => {
          const [dx, dy] = layout[j];
          this.yieldIcon(i, v, cx + dx * size, cy + dy * size + r * 0.1, size);
        });
      }
    }

    // what changed this turn
    ctx.strokeStyle = "#ffe14f";
    ctx.lineWidth = Math.max(1.5, r * 0.12);
    for (const [p, cx, cy] of visible) {
      if (!this.changed.has(key(p[0], p[1]))) continue;
      hexPath(cx, cy, r * 0.9);
      ctx.stroke();
    }

    // settlers on their way: a dashed line from the city they left to the site, the settler along it
    for (const trip of this.state?.settlers ?? []) {
      const [from, tx, ty, arrive] = trip;
      const city = this.state!.cities.find((c) => c.name === from);
      if (!city) continue;
      const [ax, ay] = S(...centre(city.xy[0], city.xy[1], this.data.height));
      const [bx, by] = S(...centre(tx, ty, this.data.height));
      ctx.strokeStyle = "#7dff9a";
      ctx.lineWidth = Math.max(1.5, r * 0.1);
      ctx.setLineDash([r * 0.3, r * 0.2]);
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
      ctx.stroke();
      hexPath(bx, by, r * 0.9);
      ctx.stroke();
      ctx.setLineDash([]);
      const start = this.settlerStarts.get(settlerKey(trip)) ?? this.state!.turn;
      const f = Math.min(1, Math.max(0, (this.state!.turn - 1 + this.progress - start + 1) / Math.max(1, arrive - start + 1)));
      const [sx, sy] = [ax + (bx - ax) * f, ay + (by - ay) * f];
      this.unit("UNIT_SETTLER", "開", "#3f9f5a", sx, sy, r * 0.32);
    }

    // builders: from where they were last turn to where they are now, with the way they went
    const before = new Map((this.prev?.builders ?? []).filter((b) => b[3] >= 0).map((b) => [b[3], b]));
    for (const [x, y, charges, id] of this.state?.builders ?? []) {
      const [nx, ny] = S(...centre(x, y, this.data.height));
      const was = id >= 0 ? before.get(id) : undefined;
      let [cx, cy] = [nx, ny];
      if (was && (was[0] !== x || was[1] !== y)) {
        const [px, py] = S(...centre(was[0], was[1], this.data.height));
        ctx.strokeStyle = "rgba(242,193,78,0.7)";
        ctx.lineWidth = Math.max(1.5, r * 0.08);
        ctx.beginPath();
        ctx.moveTo(px + r * 0.38, py + r * 0.3);
        ctx.lineTo(nx + r * 0.38, ny + r * 0.3);
        ctx.stroke();
        cx = px + (nx - px) * this.progress;
        cy = py + (ny - py) * this.progress;
      }
      this.unit("UNIT_BUILDER", "労", "#b8862e", cx + r * 0.38, cy + r * 0.3, r * 0.28);
      if (detail) {
        // the builder's charges left
        ctx.fillStyle = "#000";
        ctx.beginPath();
        ctx.arc(cx + r * 0.6, cy + r * 0.5, r * 0.13, 0, Math.PI * 2);
        ctx.fill();
        this.text(String(charges), cx + r * 0.6, cy + r * 0.51, r * 0.2, "#fff", true);
      }
    }

    // city names
    if (scale >= 3) {
      for (const c of this.state?.cities ?? []) {
        const [cx, cy] = S(...centre(c.xy[0], c.xy[1], this.data.height));
        const name = this.data.names[c.name] ?? c.name;
        const size = Math.max(10, Math.min(16, r * 0.7));
        ctx.font = `bold ${size}px sans-serif`;
        const tw = ctx.measureText(name).width;
        ctx.fillStyle = "rgba(10,12,16,0.8)";
        ctx.fillRect(cx - tw / 2 - 4, cy - r * 1.05 - size - 2, tw + 8, size + 5);
        this.text(name, cx, cy - r * 1.05 - size / 2 + 1, size, "#fff", true);
      }
    }

    if (this.hover) {
      const [cx, cy] = S(...centre(this.hover[0], this.hover[1], this.data.height));
      hexPath(cx, cy, r);
      ctx.strokeStyle = "#fff";
      ctx.lineWidth = 2;
      ctx.stroke();
    }

    if (!this.state) {
      this.text("このターンの記録はありません", w / 2, 24, 14, "#ccc");
    }
  }

  /** a unit as a disc of `colour` with the game's icon (or `letter`) on it */
  private unit(type: string, letter: string, colour: string, x: number, y: number, rr: number) {
    const ctx = this.ctx;
    ctx.fillStyle = colour;
    ctx.strokeStyle = "#000";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(x, y, rr, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    const img = icon(type);
    if (img) ctx.drawImage(img, x - rr * 0.8, y - rr * 0.8, rr * 1.6, rr * 1.6);
    else if (this.view.scale >= 5) this.text(letter, x, y + 1, rr * 1.2, "#000", true);
  }

  private yieldIcon(i: number, v: number, x: number, y: number, size: number) {
    const ctx = this.ctx;
    if (yieldAtlas.complete && yieldAtlas.naturalWidth) {
      const row = Math.min(v, 5) - 1;
      // the first column starts left of the image: draw only the part that is in it
      const sx = 128 * i - 32, sy = 128 * row;
      const cut = Math.max(0, -sx);
      const k = size / 64;
      ctx.drawImage(yieldAtlas, sx + cut, sy, 128 - cut, 128, x - 64 * k + cut * k, y - 64 * k, (128 - cut) * k, 128 * k);
      if (v >= 5) this.text(String(v), x + size * 0.28, y + size * 0.3, size * 0.55, "#fff", true);
      return;
    }
    ctx.fillStyle = YIELD_COLOURS[i];
    ctx.beginPath();
    ctx.arc(x, y, size * 0.42, 0, Math.PI * 2);
    ctx.fill();
    this.text(String(v), x, y + 1, size * 0.6, "#000", true);
  }

  private text(s: string, x: number, y: number, size: number, colour: string, bold = false) {
    const ctx = this.ctx;
    ctx.font = `${bold ? "bold " : ""}${Math.max(6, size)}px sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillStyle = colour;
    ctx.fillText(s, x, y);
  }
}

function hexA(hex: string, a: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}
