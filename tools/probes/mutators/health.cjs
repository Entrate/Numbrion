const ps=process.argv[2]||'/home/aminaliu/src/pokemon-showdown';const {Battle}=require(ps+'/dist/sim');
const packed='Pikachu|||static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric';
function fixture(){const b=new Battle({formatid:'gen9randomdoublesbattle',seed:[1,2,3,4]});b.start=()=>{};b.setPlayer('p1',{team:packed+']'+packed});b.setPlayer('p2',{team:packed+']'+packed});b.p1.foe=b.p2;b.p2.foe=b.p1;const p=b.p1.pokemon[0],t=b.p2.pokemon[0];b.p1.active=b.p1.pokemon.slice();b.p2.active=b.p2.pokemon.slice();for(const m of [...b.p1.active,...b.p2.active])m.isActive=true;b.field.pseudoWeather={};b.log=[];p.hp=50;p.maxhp=100;return {b,p,t};}
for(const op of ['damage','directDamage','heal'])for(const amount of [0,0.5,1,40,99,-1,NaN,Infinity,4294967297])for(const key of ['','uturn','tox','confusion','recoil','drain','rest','wish']) {
 const {b,p,t}=fixture();const effect=['uturn','rest'].includes(key)?b.dex.moves.get(key):key?b.dex.conditions.get(key):null;
 const r=b[op](amount,p,t,effect);
 const line=b.log.filter(x=>!x.startsWith('|split|')).map(x=>{const a=x.split('|');return [a[1],...a.slice(4)].join('|');}).join(';')||'-';
 console.log([op,String(amount),key||'-',String(r),p.hp,b.faintQueue.length,b.faintQueue[0]?.source===t?1:0,b.faintQueue[0]?.effect?.id||'-',b.prng.getSeed(),line].join('\t'));
}
