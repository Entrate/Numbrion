// Actual pinned Battle/Pokemon queries; only constructor auto-start is suspended.
// Extract with node oracle.cjs /path/to/pinned/pokemon-showdown.
const path=require('node:path'), cp=require('node:child_process');
const ps=process.argv[2]||'/home/aminaliu/src/pokemon-showdown';
const pin='7332b60e22b9e8194bb53549549eba241d73cc9a';
if(cp.execFileSync('git',['rev-parse','HEAD'],{cwd:ps,encoding:'utf8'}).trim()!==pin)throw Error('Oracle pin mismatch');
const {Battle}=require(path.join(ps,'dist/sim'));
const names=['atk','def','spa','spd','spe'];
const rows=[`# Actual pinned Pokemon methods ${pin}; no event/query mechanics substituted`];
function fixture(ability='static',item='') {
 const packed=`Pikachu||${item}|${ability}|thunderbolt,uturn|Serious||M|||100|,,,,,Electric`;
 const b=new Battle({formatid:'gen9randomdoublesbattle',seed:[1,2,3,4]});b.start=()=>{};
 b.setPlayer('p1',{team:packed});b.setPlayer('p2',{team:'Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric'});
 b.p1.foe=b.p2;b.p2.foe=b.p1;
 const p=b.p1.pokemon[0],t=b.p2.pokemon[0];b.p1.active=[p,null];b.p2.active=[t,null];p.isActive=t.isActive=true;
 return {b,p,t};
}
function stored(p,values){names.forEach((k,i)=>p.storedStats[k]=values[i]);}
function boosts(p,stage){Object.keys(p.boosts).forEach(k=>p.boosts[k]=stage);}
function num(n){return Number.isNaN(n)?'NaN':String(n);}
for(const stat of names)for(const stage of [-128,-7,-6,-5,-1,0,1,2,5,6,7,127]) {
 for(const modifier of [undefined,0,NaN,0.75,1.3,-1,Infinity])for(const other of [false,true]) {
  const {b,p,t}=fixture();stored(p,[111,222,333,444,555]);
  rows.push(['C',stat,stage,modifier===undefined?'undefined':num(modifier),Number(other),num(p.calculateStat(stat,stage,modifier,other?t:undefined)),b.prng.getSeed()].join('\t'));
 }
}
for(const ability of ['static','hugepower'])for(const item of ['', 'choiceband','assaultvest','choicescarf'])for(const active of [false,true]) {
 for(const stage of [-6,-1,0,6])for(const stat of names)for(const unboosted of [false,true])for(const unmodified of [false,true]) {
  const {b,p}=fixture(ability,item);p.isActive=active;stored(p,[111,222,333,444,555]);boosts(p,stage);
  rows.push(['G',ability,item||'-',Number(active),stage,stat,Number(unboosted),Number(unmodified),num(p.getStat(stat,unboosted,unmodified)),b.prng.getSeed()].join('\t'));
 }
}
for(const base of [0,1,4095,8191,8192,9999,10000,10001,65535])for(const stage of [-6,0,6])for(const scarf of [false,true])for(const room of [false,true]) {
 const {b,p}=fixture('static',scarf?'choicescarf':'');p.storedStats.spe=base;p.boosts.spe=stage;
 if(room)b.field.pseudoWeather.trickroom={id:'trickroom',target:b.field};
 const stat=p.getStat('spe');const action=p.getActionSpeed();p.updateSpeed();
 rows.push(['S',base,stage,Number(scarf),Number(room),num(stat),action,p.speed,b.prng.getSeed()].join('\t'));
}
for(const values of [[0,0,0,0,0],[100,100,100,100,100],[100,200,300,400,500],[500,400,300,200,100]])for(const power of [false,true])for(const unboosted of [false,true])for(const unmodified of [false,true]) {
 const {b,p}=fixture(power?'hugepower':'static',power?'choiceband':'');stored(p,values);
 rows.push(['B',values.join(','),Number(power),Number(unboosted),Number(unmodified),p.getBestStat(unboosted,unmodified),b.prng.getSeed()].join('\t'));
}
for(const weight of [0,1,55,65535])for(const heavy of [false,true])for(const active of [false,true]) {
 const {b,p}=fixture(heavy?'heavymetal':'static');p.weighthg=weight;p.isActive=active;
 rows.push(['W',weight,Number(heavy),Number(active),num(p.getWeight()),b.prng.getSeed()].join('\t'));
}
// getMoveHitData aliases slot strings, including party-position changes.
{
 const {b,p,t}=fixture();const move=b.dex.getActiveMove('uturn');
 const first=p.getMoveHitData(move);first.crit=true;first.typeMod=-2;first.bypassProtect=true;
 const same=p.getMoveHitData(move);p.position=1;const next=p.getMoveHitData(move);
 p.position=0;const restored=p.getMoveHitData(move);t.position=0;const other=t.getMoveHitData(move);
 rows.push(['H',Number(first===same),Number(first===restored),Number(first===next),Number(first===other),Number(restored.crit),restored.typeMod,Number(restored.bypassProtect),Number(next.crit),next.typeMod,Number(next.bypassProtect),Object.keys(move.moveHitData).join(','),b.prng.getSeed()].join('\t'));
}
// Integration against real Unaware callbacks: sparse keys/stat_user targets and
// two perfectly tied AnyModifyBoost holders make repeated best-stat queries draw.
for(const mode of ['source','target','both'])for(const stat of names)for(const stage of [-4,0,4])for(const other of [false,true]) {
 const {b,p,t}=fixture();p.ability=mode==='target'?'static':'unaware';t.ability=mode==='source'?'static':'unaware';
 stored(p,[111,222,333,444,555]);boosts(p,stage);b.activePokemon=p;b.activeTarget=t;
 const calculated=p.calculateStat(stat,stage,1,other?t:undefined);const queried=p.getStat(stat);
 rows.push(['A',mode,stat,stage,Number(other),calculated,queried,b.prng.getSeed()].join('\t'));
}
for(const values of [[0,0,0,0,0],[100,100,100,100,100],[100,200,300,400,500],[500,400,300,200,100]])for(const unboosted of [false,true])for(const unmodified of [false,true]) {
 const {b,p,t}=fixture('unaware');t.ability='unaware';stored(p,values);
 const actual=p.getBestStat(unboosted,unmodified);
 rows.push(['U',values.join(','),Number(unboosted),Number(unmodified),actual,b.prng.getSeed()].join('\t'));
}
process.stdout.write(rows.join('\n')+'\n');
