const { parseCiv6Save } = require(require('path').join(__dirname,'../third_party/civ6-savegame-parser'));
const g=parseCiv6Save(require('fs').readFileSync(process.argv[2]));
const ps=Array.isArray(g.players)?g.players:Object.values(g.players);
for (const p of ps.slice(0,14)) console.log(p.id ?? p.playerId, p.name ?? p.playerName ?? '', p.civilization ?? '', p.leader ?? '', p.isHuman ?? p.human ?? '');
