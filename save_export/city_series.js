// Dump every per-city yield history block in a save (Population/Food/Production/... per turn).
const { decompressCiv6Payload } = require('../third_party/civ6-savegame-parser');
const { readTimelineAt } = require('../third_party/civ6-savegame-parser/dist/internal/parse-timelines');
const fs = require('fs');
const payload = decompressCiv6Payload(fs.readFileSync(process.argv[2]));
const names = ['Population', 'Food', 'Production', 'Science', 'Gold', 'Culture', 'Faith'];
const pat = (n) => Buffer.concat([Buffer.from([n.length, 0, 0, 0]), Buffer.from(n)]);
let off = payload.indexOf(pat('Population')), n = 0;
while (off >= 0) {
  const next = payload.indexOf(pat('Population'), off + 1);
  const end = next > 0 ? next : payload.length;
  const row = {};
  for (const name of names) {
    const i = payload.indexOf(pat(name), off);
    if (i < 0 || i >= end) continue;
    const s = readTimelineAt(payload, i, name);
    if (s) row[name] = s.points.map((p) => `${p.turn}:${Math.round(p.value * 100) / 100}`).join(' ');
  }
  console.log(`#${n} @${off}`); for (const [k, v] of Object.entries(row)) console.log(`  ${k}: ${v}`);
  off = next; n++;
}
