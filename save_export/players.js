const { parseCiv6Save } = require('../third_party/civ6-savegame-parser');
const fs = require('fs');
const g = parseCiv6Save(fs.readFileSync(process.argv[2]));
const strip = (v) => JSON.stringify(v, (k, x) => (k === 'offset' ? undefined : x));
console.log('turn', g.metadata.turn, 'warnings:', strip(g.warnings).slice(0, 800));
console.log('mods:', strip(g.header?.mods ?? g.header?.enabledMods ?? Object.keys(g.header)).slice(0, 600));
const players = Array.isArray(g.players) ? g.players : Object.values(g.players);
for (const p of players.filter(p => p.human)) {
  console.log('---', p.playerId, p.name, p.civilization, p.leader);
  for (const k of ['techsResearched','currentResearch','techProgress','civicsCompleted','currentCivic','government','policiesSlotted','gold','goldPerTurn','faith','yields','unitsTrained','greatPersonPoints','eraScore'])
    console.log(' ', k, strip(p[k]).slice(0, 400));
}
