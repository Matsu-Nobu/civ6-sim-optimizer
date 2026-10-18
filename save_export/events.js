// Turn-by-turn yields plus what changed since the previous save (techs, civics, policies, units, buildings, districts, cities).
const { parseCiv6Save } = require('../third_party/civ6-savegame-parser');
const fs = require('fs'), path = require('path');
const dir = process.argv[2], pid = Number(process.argv[3] ?? 0);
const r = (x) => Math.round(x * 10) / 10;
const short = (s) => s?.replace(/^(UNIT|BUILDING|DISTRICT|PROJECT|TECH|CIVIC|POLICY|GOVERNMENT)_/, '');
const diff = (a = [], b = []) => b.filter((x) => !a.includes(x));
let prev = null;
for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.Civ6Save')).sort()) {
  let g;
  try { g = parseCiv6Save(fs.readFileSync(path.join(dir, f))); }
  catch (e) { console.log(`${f}: parse failed (${e.message.split('\n')[0].slice(0, 80)})`); continue; }
  const p = (Array.isArray(g.players) ? g.players : Object.values(g.players)).find((p) => p.playerId === pid);
  const cities = g.cities.filter((c) => c.playerId === pid);
  const units = g.units.filter((u) => u.playerId === pid);
  const cur = {
    techs: p.techsResearched, civics: p.civicsCompleted, boosts: p.techsBoosted, insp: p.civicsInspired,
    policies: p.policiesSlotted.map((x) => x.policy), gov: p.government,
    trained: Object.entries(p.unitsTrained ?? {}).flatMap(([k, n]) => Array(n).fill(k)),
    buildings: cities.flatMap((c) => c.buildings.map((b) => `${c.name}:${b.type}`)),
    districts: cities.flatMap((c) => c.districts.filter((d) => d.complete).map((d) => `${c.name}:${d.type}`)),
    cities: cities.map((c) => c.name),
  };
  const y = p.yields;
  const ev = [];
  if (prev) {
    const add = (label, xs) => xs.length && ev.push(`${label}:${xs.map(short).join(',')}`);
    add('+tech', diff(prev.techs, cur.techs)); add('+civic', diff(prev.civics, cur.civics));
    add('eureka', diff(prev.boosts, cur.boosts)); add('insp', diff(prev.insp, cur.insp));
    add('+policy', diff(prev.policies, cur.policies)); add('-policy', diff(cur.policies, prev.policies));
    if (prev.gov !== cur.gov) ev.push(`gov:${short(cur.gov)}`);
    const t = [...cur.trained]; for (const u of prev.trained) { const i = t.indexOf(u); if (i >= 0) t.splice(i, 1); } add('+unit', t);
    add('+bldg', diff(prev.buildings, cur.buildings)); add('+dist', diff(prev.districts, cur.districts)); add('+city', diff(prev.cities, cur.cities));
  }
  const cs = cities.map((c) => `${c.name}:p${c.population} ${short(c.production.current) ?? '-'}(${r(c.production.progress[c.production.current] ?? 0)})`).join(' ');
  const builders = units.filter((u) => u.type === 'UNIT_BUILDER').length;
  console.log(`T${String(g.metadata.turn).padStart(2)} g${r(p.gold)}(+${r(p.goldPerTurn)}) F${r(y.YIELD_FOOD ?? 0)} P${r(y.YIELD_PRODUCTION ?? 0)} S${r(y.YIELD_SCIENCE ?? 0)} C${r(y.YIELD_CULTURE ?? 0)} B${builders} [${short(p.currentResearch) ?? '-'}/${short(p.currentCivic) ?? '-'}] ${cs}${ev.length ? '\n     ' + ev.join('  ') : ''}`);
  prev = cur;
}
