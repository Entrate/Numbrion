// Read-only oracle extraction: node oracle.cjs /path/to/pinned/pokemon-showdown.
// Real Battle/Pokemon methods; suspend auto-start solely at the constructor boundary.
const fs = require('node:fs');
const cp = require('node:child_process');
const path = require('node:path');
const ps = process.argv[2] || '/home/aminaliu/src/pokemon-showdown';
const pin = '7332b60e22b9e8194bb53549549eba241d73cc9a';
if (cp.execFileSync('git', ['rev-parse', 'HEAD'], {cwd: ps, encoding:'utf8'}).trim() !== pin) throw Error('Oracle pin mismatch');
const {Battle} = require(path.join(ps, 'dist/sim'));
const source = fs.readFileSync(path.join(__dirname, '../../../dex/ids_generated.rs'), 'utf8');
const keys = kind => [...source.matchAll(new RegExp(`pub const ${kind}_([A-Z0-9]+):EffectId`, 'g'))].map(m => m[1].toLowerCase());
const packed = 'Pikachu||lightball|static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric';
const rows = [`# Real Showdown ${pin}; source Pokemon/Battle methods; no mechanics substituted`];
function fixture() {
 const b = new Battle({formatid:'gen9randomdoublesbattle',seed:[1,2,3,4]});
 b.start = () => {};
 b.setPlayer('p1',{team:packed}); b.setPlayer('p2',{team:packed});
 b.p1.foe=b.p2; b.p2.foe=b.p1;
 const p = b.p1.pokemon[0], t = b.p2.pokemon[0];
 b.p1.active = [p,null]; b.p2.active = [t,null];
 p.isActive = t.isActive = true;
 return {b,p,t};
}
for (const ability of [...keys('ABILITY'), '']) {
 for (const active of [false,true]) for (const transformed of [false,true]) {
  const {b,p} = fixture(); p.ability=ability;p.isActive=active;p.transformed=transformed;
  rows.push(['Q',ability||'-','lightball',Number(active),Number(transformed),Number(p.ignoringAbility()),Number(p.ignoringItem()),Number(p.hasAbility(ability)),Number(p.hasItem('lightball'))].join('\t'));
  if(b.prng.getSeed() !== '1,2,3,4') throw Error('Query drew PRNG');
 }
}
for (const item of [...keys('ITEM'), '']) for(const active of [false,true]) {
 const {p}=fixture();p.item=item;p.isActive=active;
 rows.push(['Q','static',item||'-',Number(active),0,Number(p.ignoringAbility()),Number(p.ignoringItem()),Number(p.hasAbility('static')),Number(p.hasItem(item))].join('\t'));
}
for(const base of [[],['Electric'],['Fire','Flying']]) for(const added of ['', 'Grass']) {
 for(const tera of ['', 'Water','Stellar']) for(const exclude of [false,true]) for(const pre of [false,true]) {
  const {b,p}=fixture();p.types=[...base];p.addedType=added;p.terastallized=tera;
  const result=p.getTypes(exclude,pre);
  rows.push(['T',base.join(',')||'-',added||'-',tera||'-',Number(exclude),Number(pre),result.join(','),p.types.join(',')||'-'].join('\t'));
  if(b.prng.getSeed() !== '1,2,3,4') throw Error('Type query drew PRNG');
 }
}
for(const contact of [false,true]) for(const protect of [false,true]) for(const category of ['Physical','Status']) for(const block of [false,true]) {
 const {b,p,t}=fixture(); const m=b.dex.getActiveMove('uturn');
 m.flags={...m.flags,contact,protect};m.category=category;
 rows.push(['C',Number(contact),Number(protect),category,Number(block),Number(b.checkMoveMakesContact(m,p,t,true)),Number(b.checkMoveBypassesProtect(m,p,t,block)),b.prng.getSeed()].join('\t'));
}
for(const values of [[5,-6,0,2,-2,6,-6],[-6,6,0,0,0,0,0]]) {
 const {b,p}=fixture();Object.keys(p.boosts).forEach((k,i)=>p.boosts[k]=values[i]);
 const input={spe:0,atk:3,evasion:-1,def:-2,accuracy:1,spa:-3};const output=p.getCappedBoost(input);
 rows.push(['B',values.join(','),Object.keys(output).join(','),Object.values(output).join(','),p.positiveBoosts()].join('\t'));
}
for(const hp of [undefined,0,NaN,-1,23.5,Infinity]) {
 const {p}=fixture();p.hp=50;
 rows.push(['H',hp===undefined?'undefined':String(hp),p.getUndynamaxedHP(hp)].join('\t'));
}
for(const ability of ['static','levitate']) for(const active of [false,true]) for(const flying of [false,true]) for(const ignore of [false,true]) for(const self of [false,true]) {
 const {b,p,t}=fixture();p.ability=ability;p.isActive=active;p.types=[flying?'Flying':'Electric'];
 b.activePokemon=self?p:t;b.activeMove=b.dex.getActiveMove('uturn');b.activeMove.ignoreAbility=ignore;
 for(const negate of [false,true]) {
  const result=p.isGrounded(negate);
  rows.push(['G',ability,Number(active),Number(flying),Number(ignore),Number(self),Number(negate),result===null?'null':String(Number(result))].join('\t'));
 }
}
for(const weather of ['', 'sunnyday','raindance','sandstorm','snowscape']) for(const cloud of [false,true]) for(const active of [false,true]) for(const fainted of [false,true]) for(const ending of [false,true]) {
 const {b,p,t}=fixture();b.field.weather=weather;t.ability=cloud?'cloudnine':'static';t.isActive=active;t.fainted=fainted;t.abilityState.ending=ending;
 rows.push(['W',weather||'-',Number(cloud),Number(active),Number(fainted),Number(ending),p.effectiveWeather()||'-'].join('\t'));
}
process.stdout.write(rows.join('\n')+'\n');
