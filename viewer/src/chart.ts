export interface Series {
  label: string;
  colour: string;
  points: [number, number][];
}

/** a line chart of values by turn, with the current turn marked; a click picks a turn */
export class Chart {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private series: Series[] = [];
  private turn = 0;
  private range: [number, number] = [0, 1];
  private readonly pad = { l: 48, r: 12, t: 10, b: 22 };

  constructor(parent: HTMLElement, private onPick: (turn: number) => void) {
    this.canvas = document.createElement("canvas");
    parent.appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d")!;
    this.canvas.addEventListener("click", (e) => {
      const [t0, t1] = this.range;
      const w = this.canvas.clientWidth - this.pad.l - this.pad.r;
      const t = Math.round(t0 + ((e.offsetX - this.pad.l) / w) * (t1 - t0));
      this.onPick(Math.max(t0, Math.min(t1, t)));
    });
    new ResizeObserver(() => this.draw()).observe(parent);
  }

  set(series: Series[], range: [number, number], turn: number) {
    this.series = series;
    this.range = range;
    this.turn = turn;
    this.draw();
  }

  draw() {
    const c = this.canvas, ctx = this.ctx;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const { l, r, t, b } = this.pad;
    const [t0, t1] = this.range;
    const vmax = Math.max(1, ...this.series.flatMap((s) => s.points.map((p) => p[1])));
    const vmin = Math.min(0, ...this.series.flatMap((s) => s.points.map((p) => p[1])));
    const X = (turn: number) => l + ((turn - t0) / Math.max(1, t1 - t0)) * (w - l - r);
    const Y = (v: number) => h - b - ((v - vmin) / (vmax - vmin)) * (h - t - b);
    ctx.font = "11px sans-serif";
    ctx.fillStyle = "#8a93a0";
    ctx.strokeStyle = "#2a313b";
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const v = vmin + ((vmax - vmin) * i) / 4;
      ctx.beginPath();
      ctx.moveTo(l, Y(v));
      ctx.lineTo(w - r, Y(v));
      ctx.stroke();
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      ctx.fillText(Math.round(v).toLocaleString(), l - 6, Y(v));
    }
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (let turn = Math.ceil(t0 / 10) * 10; turn <= t1; turn += 10) ctx.fillText(`T${turn}`, X(turn), h - b + 5);
    for (const s of this.series) {
      ctx.strokeStyle = s.colour;
      ctx.lineWidth = 2;
      ctx.beginPath();
      s.points.forEach(([turn, v], i) => (i ? ctx.lineTo(X(turn), Y(v)) : ctx.moveTo(X(turn), Y(v))));
      ctx.stroke();
    }
    ctx.strokeStyle = "#fff";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(X(this.turn), t);
    ctx.lineTo(X(this.turn), h - b);
    ctx.stroke();
  }
}
