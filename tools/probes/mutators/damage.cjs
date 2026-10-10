const ps=process.argv[2]||'/home/aminaliu/src/pokemon-showdown';
const {Battle}=require(ps+'/dist/sim');
const packed='Pikachu|||static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric';
function fixture() { const b=new Battle({formatid:'gen9randomdoublesbattle',seed:[1,2,3,4]});b.start=()=>{};b.setPlayer('p1',{team:packed+']'+packed});b.setPlayer('p2',{team:packed+']'+packed});b.p1.foe=b.p2;b.p2.foe=b.p1; const p=b.p1.pokemon[0],t=b.p2.pokemon[0]; b.p1.active=b.p1.pokemon.slice();b.p2.active=b.p2.pokemon.slice(); for(const m of [...b.p1.active,...b.p2.active])m.isActive=true;b.field.pseudoWeather={};b.log=[];return {b,p,t}; }
const rows=[];
for(const move of ['thunderbolt','uturn','bodypress','foulplay','psyshock','sacredsword','facade','surgingstrikes','wickedblow','nightshade','swordsdance']) {
 for(const type of ['Water','Ground','Electric','Normal','Flying','Ghost']) for(const tera of ['','Electric','Water','Stellar']) for(const crit of ['unset','true','false']) {
  const {b,p,t}=fixture(); t.types=[type];p.terastallized=tera;p.boosts.atk=-2;t.boosts.def=2;
  const m=b.dex.getActiveMove(move);if(crit!=='unset')m.willCrit=crit==='true';m.hit=1;b.activeMove=m;b.activePokemon=p;
  const damage=b.actions.getDamage(p,t,m);const hit=t.getMoveHitData(m);
  rows.push(['D',move,type,tera||'-',crit,String(damage),Number(hit.crit),hit.typeMod,b.prng.getSeed(),b.log.map(x=>x.split('|')[1]).join(',')||'-',p.stellarBoostedTypes.join(',')||'-'].join('\t'));
 }
}
for(const bp of [0,.5,40,100,150,65535,NaN,Infinity]) for(const boost of [-6,0,6]) {
 const {b,p,t}=fixture();p.boosts.atk=boost;p.boosts.def=-boost;
 const damage=b.actions.getDamage(p,t,bp);rows.push(['N',String(bp),boost,String(damage),b.prng.getSeed()].join('\t'));
 const c=fixture();c.p.boosts.atk=boost;c.p.boosts.def=-boost;rows.push(['C',String(bp),boost,c.b.actions.getConfusionDamage(c.p,bp),c.b.prng.getSeed()].join('\t'));
}
for(const move of ['uturn','thunderbolt'])for(const spread of [false,true])for(const burn of [false,true])for(const item of ['','lifeorb','expertbelt','choiceband'])for(const ability of ['static','hugepower','adaptability','sniper']) {
 const {b,p,t}=fixture();p.item=item;p.ability=ability;p.status=burn?'brn':'';t.types=['Water'];const m=b.dex.getActiveMove(move);m.spreadHit=spread;b.activeMove=m;b.activePokemon=p;
 const d=b.actions.getDamage(p,t,m);const hit=t.getMoveHitData(m);rows.push(['M',move,Number(spread),Number(burn),item||'-',ability,d,Number(hit.crit),hit.typeMod,b.prng.getSeed()].join('\t'));
}
process.stdout.write(rows.join('\n')+'\n');
