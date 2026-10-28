// The game's map is pointy-top hexes in rows; odd rows are shifted half a hex to the east, and y grows northward
// (optimizer/src/world.rs, Map::dirs). On the screen y grows downward, so rows are drawn from the top as height-1-y.

export const SQRT3 = Math.sqrt(3);

/** the centre of plot (x, y) in map units (hex radius 1) */
export function centre(x: number, y: number, height: number): [number, number] {
  return [x * SQRT3 + (y & 1 ? SQRT3 / 2 : 0) + SQRT3 / 2, (height - 1 - y) * 1.5 + 1];
}

/** the corners of a hex of radius r at (cx, cy), from the top clockwise */
export function corners(cx: number, cy: number, r: number): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i < 6; i++) {
    const a = ((-90 + 60 * i) * Math.PI) / 180;
    out.push([cx + r * Math.cos(a), cy + r * Math.sin(a)]);
  }
  return out;
}

/** the neighbour of (x, y) across edge e (0 = north-east, then clockwise), without wrapping */
export function across(x: number, y: number, e: number): [number, number] {
  const odd = y & 1;
  switch (e) {
    case 0: return [x + odd, y + 1];
    case 1: return [x + 1, y];
    case 2: return [x + odd, y - 1];
    case 3: return [x - 1 + odd, y - 1];
    case 4: return [x - 1, y];
    default: return [x - 1 + odd, y + 1];
  }
}

/** the plot under map point (mx, my): the nearest centre among the candidates of its row band */
export function plotAt(mx: number, my: number, width: number, height: number): [number, number] | null {
  const row = Math.round((my - 1) / 1.5);
  let best: [number, number] | null = null;
  let bestD = Infinity;
  for (let r = row - 1; r <= row + 1; r++) {
    const y = height - 1 - r;
    if (y < 0 || y >= height) continue;
    const x0 = Math.floor((mx - SQRT3 / 2 - (y & 1 ? SQRT3 / 2 : 0)) / SQRT3);
    for (let x = x0 - 1; x <= x0 + 1; x++) {
      if (x < 0 || x >= width) continue;
      const [cx, cy] = centre(x, y, height);
      const d = (cx - mx) ** 2 + (cy - my) ** 2;
      if (d < bestD) {
        bestD = d;
        best = [x, y];
      }
    }
  }
  return best && bestD <= 1 ? best : null;
}
