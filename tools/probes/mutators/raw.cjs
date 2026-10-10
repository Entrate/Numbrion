// Pinned prototype probes deliberately bypass the battle lifecycle.
const {Pokemon} = require(process.env.SHOWDOWN_PATH || '/home/aminaliu/src/pokemon-showdown/dist/sim/pokemon');
const {Dex} = require('/home/aminaliu/src/pokemon-showdown/dist/sim/dex');
for (const op of ['damage','heal','sethp']) for (const n of [0, .5, 12.9, 150, -1, NaN, Infinity, 4294967297]) {
 const p = {hp:100,maxhp:120,battle:{trunc:Dex.trunc,faintQueue:[]},faint:Pokemon.prototype.faint};
 const ret = Pokemon.prototype[op].call(p,n);
 console.log([op,String(n),String(ret),p.hp,p.battle.faintQueue.length].join('\t'));
}
