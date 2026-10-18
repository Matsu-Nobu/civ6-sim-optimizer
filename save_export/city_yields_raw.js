// Find per-city yield maps in a save: `6, (yieldHash, value*256) x6`, the layout CivVIReplay reads
// (src/CivVIParser.cpp, after the city id / 0x11 header). Prints each hit with the preceding city id.
const { decompressCiv6Payload } = require('../third_party/civ6-savegame-parser');
const { civ6Hash } = require('../third_party/civ6-savegame-parser/dist/internal/hash-tables');
const fs = require('fs');
const YIELDS = ['YIELD_FOOD', 'YIELD_PRODUCTION', 'YIELD_GOLD', 'YIELD_SCIENCE', 'YIELD_CULTURE', 'YIELD_FAITH'];
const byHash = new Map(YIELDS.map((y) => [civ6Hash(y) >>> 0, y.slice(6, 7)]));
const buf = decompressCiv6Payload(fs.readFileSync(process.argv[2]));
const readMap = (i) => {
  if (buf.readUInt32LE(i) !== 6) return null;
  const m = {};
  for (let k = 0; k < 6; k++) {
    const h = buf.readUInt32LE(i + 4 + k * 8), v = buf.readInt32LE(i + 8 + k * 8);
    if (!byHash.has(h)) return null;
    m[byHash.get(h)] = v / 256;
  }
  return m;
};
for (let i = 0; i + 60 < buf.length; i++) {
  if (buf.readUInt32LE(i) !== 6) continue;
  const a = readMap(i);
  if (!a) continue;
  const b = readMap(i + 52);
  // header per CivVIReplay: id, 0x11, ?, ?, ?, 0, ? then the two maps (28 bytes before)
  const id = buf.readUInt32LE(i - 28), tag = buf.readUInt32LE(i - 24);
  const fmt = (m) => m ? Object.entries(m).map(([k, v]) => `${k}${v}`).join(' ') : '-';
  console.log(`@${i} id=${id} tag=${tag.toString(16)} | ${fmt(a)} || ${fmt(b)}`);
  i += 51;
}
