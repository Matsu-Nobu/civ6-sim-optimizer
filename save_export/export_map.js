// Export the whole map (terrain, feature, resource, river, owner) from one save as JSON.
// Usage: save_export/export_map.sh <save> <rules.sqlite> > map.json   (sets TYPES_FILE / RESOURCES like export_game.sh;
// without them, resources the parser's dictionary lacks, such as BBG's maple, come out empty)
const { parseCiv6Save } = require('../third_party/civ6-savegame-parser');
const { civ6Hash, registerTypes } = require('../third_party/civ6-savegame-parser/dist/internal/hash-tables');
const fs = require('fs');
if (process.env.TYPES_FILE) registerTypes(JSON.parse(fs.readFileSync(process.env.TYPES_FILE, 'utf8')));
const RESOURCES = new Map((process.env.RESOURCES || '').split(',').filter(Boolean).map((n) => [civ6Hash(n) >>> 0, n]));
const g = parseCiv6Save(fs.readFileSync(process.argv[2]));
const plots = g.map.plots.map((p) => ({
  x: p.x, y: p.y, terrain: p.terrain, feature: p.feature,
  resource: p.resource ?? (p.resourceHash != null ? RESOURCES.get(p.resourceHash >>> 0) ?? null : null), improvement: p.improvement,
  river: p.riverMap !== 0, riverEdges: p.riverMap, route: p.routeIndex ?? null, owner: p.owner?.playerId ?? null, yieldChanges: p.yieldChanges ?? null,
}));
console.log(JSON.stringify({ turn: g.metadata.turn, width: g.map.width, height: g.map.height, plots }));
