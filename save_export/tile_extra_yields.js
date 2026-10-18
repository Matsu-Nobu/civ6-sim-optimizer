// Per-tile extra yields, read the way CivVIReplay does (src/CivVIParser.cpp, "Feature" anchor):
// "Feature", +4, +8, map, tileCount, i32 index[tileCount], count, count x map{n, (yieldHash, u32 amount) x n}.
const { decompressCiv6Payload } = require('../third_party/civ6-savegame-parser');
const { civ6Hash } = require('../third_party/civ6-savegame-parser/dist/internal/hash-tables');
const fs = require('fs');
const NAMES = ['YIELD_FOOD', 'YIELD_PRODUCTION', 'YIELD_GOLD', 'YIELD_SCIENCE', 'YIELD_CULTURE', 'YIELD_FAITH'];
const BY_HASH = new Map(NAMES.map((n) => [civ6Hash(n) >>> 0, n]));

function tileExtraYields(buf, tileCount) {
  let i = buf.indexOf(Buffer.from('Feature'));
  if (i < 0) return null;
  i += 7 + 4 + 8;
  const readMap = () => {
    const n = buf.readUInt32LE(i); i += 4;
    const m = {};
    for (let k = 0; k < n; k++) { const key = buf.readUInt32LE(i), v = buf.readInt32LE(i + 4); i += 8; m[BY_HASH.get(key) ?? key.toString(16)] = v; }
    return m;
  };
  readMap();
  const tc = buf.readUInt32LE(i); i += 4;
  if (tc !== tileCount) throw new Error(`tile count ${tc} != ${tileCount}`);
  const index = [];
  for (let t = 0; t < tileCount; t++) { index.push(buf.readInt32LE(i)); i += 4; }
  const count = buf.readUInt32LE(i); i += 4;
  const maps = [];
  for (let e = 0; e < count; e++) maps.push(readMap());
  const out = new Map();
  index.forEach((e, t) => { if (e >= 0 && maps[e]) out.set(t, maps[e]); });
  return out;
}
module.exports = { tileExtraYields };

if (require.main === module) {
  const { parseCiv6Save } = require('../third_party/civ6-savegame-parser');
  const raw = fs.readFileSync(process.argv[2]);
  const g = parseCiv6Save(raw);
  const extra = tileExtraYields(decompressCiv6Payload(raw), g.map.plots.length);
  console.log('tiles with extras:', extra.size);
  for (const [t, m] of extra) {
    const p = g.map.plots[t];
    if (p.owner?.playerId !== 0) continue;
    const nz = Object.fromEntries(Object.entries(m).filter(([, v]) => v));
    if (Object.keys(nz).length) console.log(p.x, p.y, p.terrain, p.feature, p.resource, p.improvement, JSON.stringify(nz));
  }
}
