const ps = process.argv[2] || '/home/aminaliu/src/pokemon-showdown';
const {Battle} = require(ps + '/dist/sim');
const packed = 'Pikachu||choicescarf|static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric';
function fixture() {
 const b = new Battle({formatid:'gen9randomdoublesbattle',seed:[1,2,3,4]}); b.start=()=>{};
 b.setPlayer('p1',{team:packed+']'+packed}); b.setPlayer('p2',{team:packed+']'+packed}); b.p1.foe=b.p2; b.p2.foe=b.p1;
 const p=b.p1.pokemon[0],t=b.p2.pokemon[0]; b.p1.active=[p,b.p1.pokemon[1]];b.p2.active=[t,b.p2.pokemon[1]]; for(const m of [...b.p1.active,...b.p2.active])m.isActive=true;
 b.field.pseudoWeather={}; b.log=[]; return {b,p,t};
}
function result(r) { return r && typeof r === 'object' ? r.id : String(r); }
function row(op,key,{b,p},r,extra='-') { console.log([op,key,result(r),p.status||'-',p.hp,p.item||'-',p.lastItem||'-',p.ability||'-',b.effectOrder,b.prng.getSeed(),Object.keys(p.volatiles).join(',')||'-',extra,b.log.filter(x=>!x.startsWith('|split|')).map(x=>x.split('|')[1]).join(',')||'-'].join('\t')); }
for(const s of ['brn','par','slp','frz','psn','tox']) { const f=fixture(); row('status',s,f,f.p.setStatus(s,f.t,f.b.dex.moves.get('thunderbolt'))); }
for(const s of ['brn','slp']) {const f=fixture();f.p.setStatus(s); row('cure',s,f,f.p.cureStatus(false));}
for(const s of ['confusion','protect','substitute','taunt']) { const f=fixture(); const r=f.p.addVolatile(s,f.t,f.b.dex.moves.get('thunderbolt'));row('volatile',s,f,r, f.p.volatiles[s]?.duration ?? '-'); }
for(const s of ['tailwind','reflect','lightscreen']) {const f=fixture();row('side',s,f,f.p.side.addSideCondition(s,f.p), f.p.side.sideConditions[s]?.duration ?? '-');}
for(const s of ['raindance','sunnyday','snowscape','sandstorm']) {const f=fixture();row('weather',s,f,f.b.field.setWeather(s,f.p),f.b.field.weatherState.duration ?? '-');}
for(const s of ['electricterrain','grassyterrain','mistyterrain','psychicterrain']) {const f=fixture();row('terrain',s,f,f.b.field.setTerrain(s,f.p),f.b.field.terrainState.duration ?? '-');}
for(const s of ['trickroom','gravity']) {const f=fixture();row('pseudo',s,f,f.b.field.addPseudoWeather(s,f.p),f.b.field.pseudoWeather[s]?.duration ?? '-');}
for(const op of ['take','set','clear','use']) {const f=fixture();const r=op==='take'?f.p.takeItem(f.t):op==='set'?f.p.setItem('leftovers'):op==='clear'?f.p.clearItem():f.p.useItem();row(op,'-',f,r);}
for(const ability of ['levitate','hugepower','']) {const f=fixture();row('ability',ability||'-',f,f.p.setAbility(ability,f.t,f.b.dex.moves.get('skillswap')));}
for(const a of ['static','contrary','simple','clearbody']) {const f=fixture();f.p.ability=a;row('boost',a,f,f.b.boost({def:-1,spd:-1,atk:2,spa:2,spe:2},f.p,f.t,f.b.dex.moves.get('shellsmash')),Object.values(f.p.boosts).join(','));}
