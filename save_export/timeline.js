const { parseCiv6Save } = require('../third_party/civ6-savegame-parser');
const fs = require('fs'), path = require('path');
const dir = process.argv[2], pid = Number(process.argv[3] ?? 0);
const files = fs.readdirSync(dir).filter(f => f.endsWith('.Civ6Save')).sort();
for (const f of files) {
  const g = parseCiv6Save(fs.readFileSync(path.join(dir, f)));
  const p = (Array.isArray(g.players) ? g.players : Object.values(g.players)).find(p => p.playerId === pid);
  const y = p.yields, r = (x) => Math.round(x * 10) / 10;
  const cities = g.cities.filter(c => c.playerId === pid).map(c => {
    const cur = c.production.current?.replace(/^(UNIT|BUILDING|DISTRICT|PROJECT)_/, '');
    return `${c.name}:pop${c.population} food${r(c.food)} ${cur}(${r(c.production.progress[c.production.current] ?? 0)})`;
  });
  console.log(`T${g.metadata.turn} gold${r(p.gold)}(+${r(p.goldPerTurn)}) F${r(y.YIELD_FOOD)} P${r(y.YIELD_PRODUCTION)} S${r(y.YIELD_SCIENCE)} C${r(y.YIELD_CULTURE)} res=${p.currentResearch?.slice(5)} civ=${p.currentCivic?.slice(6)} | ${cities.join(' | ')}`);
}
