const P=require('path').join(__dirname,'../third_party/civ6-savegame-parser');
const { parseCiv6Save, decompressCiv6Payload } = require(P);
const { civ6Hash } = require(P+'/dist/internal/hash-tables');
const fs=require('fs'), path=require('path');
const YN=['YIELD_FOOD','YIELD_PRODUCTION','YIELD_GOLD','YIELD_SCIENCE','YIELD_CULTURE','YIELD_FAITH'];
const YB=new Map(YN.map(y=>[civ6Hash(y)>>>0,y]));
function runs(buf){const readMap=(i)=>{if(buf.readUInt32LE(i)!==6)return null;const m={};for(let k=0;k<6;k++){const y=YB.get(buf.readUInt32LE(i+4+k*8));if(!y)return null;m[y]=buf.readInt32LE(i+8+k*8)/256;}return m;};
 const out=[];for(let i=28;i+110<buf.length;i++){if(buf.readUInt32LE(i)!==6)continue;const tag=buf.readUInt32LE(i-24);if(tag!==0x10&&tag!==0x11)continue;const a=readMap(i),b=a&&readMap(i+52);if(!b)continue;const id=buf.readUInt32LE(i-28);if(id===0)out.push([]);if(out.length&&id===out[out.length-1].length)out[out.length-1].push(a);i+=103;}return out;}
const dir=process.argv[2];
for (const f of fs.readdirSync(dir).filter(f=>f.endsWith('.Civ6Save')).sort()) {
  const raw=fs.readFileSync(path.join(dir,f)); let g; try { g=parseCiv6Save(raw); } catch(e) { continue; }
  const R=runs(decompressCiv6Payload(raw));
  const ids=[...new Set(g.cities.map(c=>c.playerId))].sort((a,b)=>a-b);
  const row={turn:g.turn ?? g.gameTurn ?? f, players:{}};
  ids.forEach((pid,k)=>{ const r=R[k]||[]; const n=g.cities.filter(c=>c.playerId===pid).length; if(r.length!==n) return;
    row.players[pid]={cities:n, fp:r.reduce((s,m)=>s+m.YIELD_FOOD+m.YIELD_PRODUCTION,0), sci:r.reduce((s,m)=>s+m.YIELD_SCIENCE,0), cul:r.reduce((s,m)=>s+m.YIELD_CULTURE,0)}; });
  console.log(JSON.stringify(row));
}
