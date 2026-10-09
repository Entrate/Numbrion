#!/usr/bin/env node
// Node 24, no packages. The checkout is read-only; all output goes to this repo.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import {COMMIT,assertOraclePin} from './oracle.mjs';
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const PS = path.resolve(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const OUT = process.argv[3] ? path.resolve(process.argv[3]) : path.join(ROOT, 'crates/engine/src/dex');
assertOraclePin(PS);
const {Dex} = createRequire(path.join(PS, 'package.json'))('./dist/sim');
const dex = Dex.forFormat('gen9randomdoublesbattle');
if (dex !== Dex.mod('gen9')) throw Error('Unexpected dex mod');
const scopeBytes = fs.readFileSync(path.join(ROOT,'data/scope.json'));
const scope = JSON.parse(scopeBytes);
if (scope.meta.showdownCommit !== COMMIT) throw Error('Wrong scope commit');
const hash = s => crypto.createHash('sha256').update(s).digest('hex');
const q = s => JSON.stringify(s).replace(/\\u([0-9a-f]{4})/gi, (_,h)=>`\\u{${h}}`);
const id = s => String(s || '').toLowerCase().replace(/[^a-z0-9]/g,'');
const symbol = s => s.replace(/[^a-zA-Z0-9]/g,'_').toUpperCase();
const kinds = ['species','moves','abilities','items','conditions','rules'];
const singular = ['SPECIES','MOVE','ABILITY','ITEM','CONDITION','RULE'];
const objects = {}, ids = {}, ranges = {}; let next = 1;
for (let i=0;i<kinds.length;i++) {
 const k=kinds[i]; objects[k]=[...scope[k]].sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0).map(e=>({id:e.id, value:k==='rules'?dex.formats.get(e.id):dex[k].get(e.id)}));
 ranges[k]=[next,next+objects[k].length]; ids[k]=new Map(objects[k].map(e=>[e.id,next++]));
}
const ref = (kind,s) => `EffectId(${ids[kind].get(id(s)) || 0})`;
const types = ['Normal','Fighting','Flying','Poison','Ground','Rock','Bug','Ghost','Steel','Fire','Water','Grass','Electric','Psychic','Ice','Dragon','Dark','Fairy','Stellar','???'];
const type = s => `TypeId(${s ? types.indexOf(s)+1 : 0})`;
const stats = ['hp','atk','def','spa','spd','spe'];
const targets = ['normal','self','any','allAdjacentFoes','allySide','allAdjacent','adjacentAlly','all','allies','foeSide','randomNormal'];
const targetNames = ['Normal','SelfTarget','Any','AllAdjacentFoes','AllySide','AllAdjacent','AdjacentAlly','All','Allies','FoeSide','RandomNormal'];
// Lossless declarative trees supplement typed hot data. Keys use integer FieldId; JS object order is retained.
const fields = new Set();
function data(v, key='') {
 if (typeof v==='function'||v===undefined || key==='condition') return undefined;
 if (v===null||typeof v!=='object') return v;
 if (Array.isArray(v)) return v.map(x=>data(x)).filter(x=>x!==undefined);
 const out={};for(const[k,x]of Object.entries(v)) {const value=data(x,k);if(value!==undefined){fields.add(k);out[k]=value;}}return out;
}
for(const k of kinds)for(const e of objects[k])e.data=data(e.value);
const fieldKeys=[...fields].sort(); const fieldMap=new Map(fieldKeys.map((f,i)=>[f,i]));
function value(v){
 if(v===null)return 'DataValue::Null';
 if(typeof v==='boolean')return `DataValue::Bool(${v})`;
 if(typeof v==='number')return `DataValue::Number(${Number.isInteger(v)?v+'.0':v})`;
 if(typeof v==='string')return `DataValue::Text(${q(v)})`;
 if(Array.isArray(v))return `DataValue::Array(&[${v.map(value).join(',')}])`;
 return `DataValue::Object(&[${Object.entries(v).map(([k,x])=>`Property{key:FieldId(${fieldMap.get(k)}),value:${value(x)}}`).join(',')}])`;
}
const flagKeys=[...new Set(kinds.flatMap(k=>objects[k].flatMap(e=>Object.keys(e.value.flags||{}))))].sort();
if(flagKeys.length>64)throw Error('Flags exceeded u64');
const flags = o => '0x'+Object.entries(o||{}).reduce((bits,[k,v])=>v?bits|(1n<<BigInt(flagKeys.indexOf(k))):bits,0n).toString(16);
let idsBody='use crate::ids::EffectId;\n';
for(let i=0;i<kinds.length;i++){
 const k=kinds[i],p=singular[i];idsBody+=`pub const ${p}_START:u16=${ranges[k][0]};\npub const ${p}_END:u16=${ranges[k][1]};\npub const ${p}_COUNT:usize=${objects[k].length};\n`;
 for(const e of objects[k])idsBody+=`pub const ${p}_${symbol(e.id)}:EffectId=EffectId(${ids[k].get(e.id)});\n`;
}
idsBody+=`pub const EFFECT_COUNT:usize=${next};\n`;
// Nested secondary and secondaries are both retained, because Dex exposes both; callers choose their actual site.
const events=new Set(['EntryHazard','ModifyAtk','ModifyDef','ModifySpA','ModifySpD','ModifySpe','SideResidual','FieldResidual','SideSwitchIn','FieldSwitchIn']);
const directKeys=new Set(['basePowerCallback','damageCallback','durationCallback','beforeMoveCallback','beforeTurnCallback','priorityChargeCallback','effect']);
const directNames={basePowerCallback:'BasePowerCallback',damageCallback:'DamageCallback',durationCallback:'DurationCallback',beforeMoveCallback:'BeforeMoveCallback',beforeTurnCallback:'BeforeTurnCallback',priorityChargeCallback:'PriorityChargeCallback',effect:'FlingEffect'};
const hooks=[];const manifests=[];
function decode(k){
 if(directKeys.has(k))return [directNames[k],'Direct'];
 let name=k.slice(2),rel='On';for(const r of ['Source','Ally','Foe','Any'])if(name.startsWith(r)){name=name.slice(r.length);rel=r;break;}return[name,rel];
}
function walkHooks(obj,effect,site=''){
 if(!obj||typeof obj!=='object')return;
 for(const[k,v]of Object.entries(obj)){
  if(k==='condition')continue;
  // Metadata-only onSideResidualOrder must also survive for duration-only residual collection.
  const on=/^on[A-Z]/.test(k)&&k!=='onPlate';
  const suffix=/(?:SubOrder|Order|Priority)$/.test(k);
  const metadata=on&&suffix&&Object.keys(obj).some(x=>x!==k&&k===x+'Priority'||k===x+'Order'||k===x+'SubOrder');
  // onModifyPriority is an event. Strip metadata only if suffix owner exists, or Order/SubOrder ends it.
  const isMeta=on && (metadata || /(?:SubOrder|Order)$/.test(k) || (k.endsWith('Priority')&&k!=='onModifyPriority'&&k!=='onFractionalPriority'));
  if((on&&!isMeta&&v!==undefined)||directKeys.has(k)&&typeof v==='function'){
   const[event,rel]=decode(k);events.add(event);
   hooks.push({effect,key:k,site,event,rel,order:obj[k+'Order']||0,priority:obj[k+'Priority']||0,sub:obj[k+'SubOrder']||0,callback:v,synthetic:false});
  } else if(isMeta) {
   const base=k.replace(/(?:SubOrder|Order|Priority)$/,'');
   if(obj[base]===undefined&&!hooks.some(h=>h.effect===effect&&h.site===site&&h.key===base)){
    const[event,rel]=decode(base);events.add(event);hooks.push({effect,key:base,site,event,rel,order:obj[base+'Order']||0,priority:obj[base+'Priority']||0,sub:obj[base+'SubOrder']||0,callback:undefined,synthetic:true});
   }
  }
  if(v&&typeof v==='object'&&['self','selfBoost','secondary','secondaries','fling'].includes(k)){
   if(Array.isArray(v))v.forEach((x,i)=>walkHooks(x,effect,site?`${site}.${k}.${i}`:`${k}.${i}`));else walkHooks(v,effect,site?`${site}.${k}`:k);
  }
 }
}
for(const k of kinds)for(const e of objects[k]){
 const start=hooks.length;walkHooks(e.value,ids[k].get(e.id));manifests.push({effect:ids[k].get(e.id),start,len:hooks.length-start,duration:e.value.duration,durationCallback:typeof e.value.durationCallback==='function',type:e.value.effectType||'Condition'});
}
// Source call sites, rather than only handler keys: includes zero-listener events and dynamic stat names.
for(const f of fs.readdirSync(path.join(PS,'sim')).filter(f=>f.endsWith('.ts')).sort()){
 const source=fs.readFileSync(path.join(PS,'sim',f),'utf8');
 for(const m of source.matchAll(/\b(?:singleEvent|runEvent|priorityEvent|eachEvent|fieldEvent)\s*\(\s*['"]([A-Za-z][A-Za-z0-9]*)['"]\s*[,)]/g))events.add(m[1]);
}
for(const h of hooks)if(typeof h.callback==='function')for(const m of h.callback.toString().matchAll(/\b(?:singleEvent|runEvent|priorityEvent|eachEvent|fieldEvent)\s*\(\s*['"]([A-Za-z][A-Za-z0-9]*)['"]\s*[,)]/g))events.add(m[1]);
// raw singleEvent('FoeMaybeTrapPokemon') is distinct from relationship-prefixed runEvent('MaybeTrapPokemon').
const eventNames=[...events].sort();
let eventBody=`#[derive(Clone,Copy,Debug,PartialEq,Eq,Hash)]\n#[repr(u16)]\npub enum EventId{${eventNames.map((e,i)=>`${e}=${i}`).join(',')}}\npub const EVENT_COUNT:usize=${eventNames.length};\npub const EVENT_NAMES:[&str;EVENT_COUNT]=[${eventNames.map(q).join(',')}];\n`;
const hookValue=h=>h.synthetic?'HookValue::Absent':typeof h.callback==='function'?'HookValue::Function':`HookValue::Constant(${value(h.callback)})`;
let hookBody='use super::*;\npub static HOOKS:&[Hook]=&[\n'+hooks.map(h=>`Hook{effect:EffectId(${h.effect}),key:${q(h.key)},site:${q(h.site)},event:EventId::${h.event},rel:HookRel::${h.rel},order:${h.order}.0,priority:${Number.isInteger(h.priority)?h.priority+'.0':h.priority},sub_order:${h.sub}.0,value:${hookValue(h)}},`).join('\n')+'\n];\n';
const effectSymbols = new Map(kinds.flatMap((k,i)=>objects[k].map(e=>[ids[k].get(e.id),singular[i]+'_'+symbol(e.id)])));
for (const [i,h] of hooks.entries()) hookBody+=`pub const HOOK_${effectSymbols.get(h.effect)}_${h.site?symbol(h.site)+'_':''}${symbol(h.key)}:HookId=HookId(${i});\n`;
hookBody+='pub static MANIFESTS:&[EffectManifest]=&[EffectManifest::EMPTY,\n'+manifests.map(m=>`EffectManifest{hooks_start:${m.start},hooks_len:${m.len},duration:${m.duration===undefined?'None':`Some(${m.duration})`},duration_callback:${m.durationCallback},effect_type:EffectType::${m.type},events:[${Array.from({length:Math.ceil(eventNames.length/64)},(_,i)=>'0x'+hooks.slice(m.start,m.start+m.len).reduce((bits,h)=>Math.floor(eventNames.indexOf(h.event)/64)===i?bits|1n<<BigInt(eventNames.indexOf(h.event)%64):bits,0n).toString(16)).join(',')}]},`).join('\n')+'\n];\n';
const traitKeys=['ignoreNegativeOffensive','ignorePositiveDefensive','ignoreOffensive','ignoreDefensive','ignoreEvasion','ignoreAbility','spreadHit','forceSTAB','hasSheerForceBoost','stallingMove','smartTarget','breaksProtect','willCrit','hasCrashDamage','thawsTarget','multiaccuracy','struggleRecoil'];
const tagKeys=[...new Set(objects.species.flatMap(e=>e.value.tags||[]))].sort();
const refs=(k,v)=>'&['+(Array.isArray(v)?v:v?[v]:[]).map(x=>ref(k,x))+']';
const statNames={hp:'Hp',atk:'Atk',def:'Def',spa:'SpA',spd:'SpD',spe:'Spe',accuracy:'Accuracy',evasion:'Evasion'};
const ratio=v=>v?`Some([${v}])`:'None';
function moveEffects(o,eid,site=''){
 const nested=o.self?`Some(&${moveEffects(o.self,eid,site?site+'.self':'self')})`:'None';
 const hookIds=hooks.flatMap((h,i)=>h.effect===eid&&h.site===site?[`HookId(${i})`]:[]);
 return `MoveEffects{boosts:&[${Object.entries(o.boosts||{}).map(([s,n])=>`BoostChange{stat:StatId::${statNames[s]},delta:${n}}`)}],status:${ref('conditions',o.status)},volatile_status:${ref('conditions',o.volatileStatus)},side_condition:${ref('conditions',o.sideCondition)},slot_condition:${ref('conditions',o.slotCondition)},weather:${ref('conditions',o.weather)},terrain:${ref('conditions',o.terrain)},pseudo_weather:${ref('conditions',o.pseudoWeather)},heal:${ratio(o.heal)},force_switch:${!!o.forceSwitch},self_switch:SelfSwitch::${o.selfSwitch===true?'Switch':o.selfSwitch==='copyvolatile'?'CopyVolatile':o.selfSwitch==='shedtail'?'ShedTail':'None'},self_effect:${nested},hooks:&[${hookIds}]}`;
}
let dataBody='use super::*;\n';
dataBody+=`pub const ORACLE_COMMIT:&str=${q(COMMIT)};\npub const SCOPE_SHA256:&str=${q(hash(scopeBytes))};\n`;
dataBody+=`pub const CALLBACKS_SHA256:&str=${q(hash(hooks.filter(h=>typeof h.callback==='function').map(h=>`${h.effect}:${h.site}:${h.key}:${h.callback.toString()}`).join('\n')))};\n`;
dataBody+=`pub const TYPE_NAMES:[&str;20]=[${types.map(q).join(',')}];\npub const FLAG_NAMES:[&str;${flagKeys.length}]=[${flagKeys.map(q).join(',')}];\n`;
for(const[f,i]of flagKeys.map((f,i)=>[f,i]))dataBody+=`pub const FLAG_${symbol(f)}:u64=1u64<<${i};\n`;
for(const[f,i]of traitKeys.map((f,i)=>[f,i]))dataBody+=`pub const MOVE_TRAIT_${symbol(f)}:u32=1u32<<${i};\n`;
for(const[f,i]of tagKeys.map((f,i)=>[f,i]))dataBody+=`pub const SPECIES_TAG_${symbol(f)}:u32=1u32<<${i};\n`;
dataBody+=`pub const FIELD_NAMES:[&str;${fieldKeys.length}]=[${fieldKeys.map(q).join(',')}];\n`;
const symbols=new Set();for(const[f,i]of fieldKeys.map((f,i)=>[f,i])) {let name=symbol(f);if(/^\d/.test(name))name='N_'+name;if(symbols.has(name))name+='_'+i;symbols.add(name);dataBody+=`pub const FIELD_${name}:FieldId=FieldId(${i});\n`;}
for(const k of kinds)dataBody+=`pub static ${k.toUpperCase()}_DATA:&[EffectData]=&[\n${objects[k].map(e=>`EffectData{id:${ref(k,e.id)},key:${q(e.id)},name:${q(e.value.name||e.id)},data:${value(e.data)}},`).join('\n')}\n];\n`;
dataBody+='pub static SPECIES:&[SpeciesData]=&[\n'+objects.species.map(e=>{const s=e.value;return `SpeciesData{id:${ref('species',e.id)},name:${q(s.name)},base_species_name:${q(s.baseSpecies)},base_species:${ref('species',s.baseSpecies)},types:[${[s.types[0],s.types[1]].map(type)}],base_stats:[${stats.map(x=>s.baseStats[x])}],abilities:[${['0','1','H','S'].map(x=>`AbilitySlot{id:${ref('abilities',s.abilities[x])},key:${q(id(s.abilities[x]))},name:${q(s.abilities[x]||'')}}`)}],gender:${q(s.gender||'')},male_ratio:${s.genderRatio.M===0?'0.0':s.genderRatio.M===1?'1.0':s.genderRatio.M},weighthg:${s.weighthg},max_hp:${s.maxHP||0},tags:${(s.tags||[]).reduce((bits,k)=>bits|1<<tagKeys.indexOf(k),0)},battle_only:${refs('species',s.battleOnly)},changes_from:${ref('species',s.changesFrom)},required_items:${refs('items',s.requiredItems||s.requiredItem)},required_ability:${ref('abilities',s.requiredAbility)},required_move:${ref('moves',s.requiredMove)},required_tera_type:${type(s.requiredTeraType)}},`;}).join('\n')+'\n];\n';
dataBody+='pub static MOVES:&[MoveData]=&[\n'+objects.moves.map(e=>{const m=e.value,eid=ids.moves.get(e.id);
const multi=Array.isArray(m.multihit)?m.multihit:[m.multihit||1,m.multihit||1];
const traits=traitKeys.reduce((bits,k,i)=>m[k]?bits|1<<i:bits,0);
const secondaries=(m.secondaries||[]).map((s,i)=>`SecondaryData{chance:${s.chance===undefined?'None':`Some(${s.chance})`},effects:${moveEffects(s,eid,`secondaries.${i}`)}}`);
return `MoveData{id:${ref('moves',e.id)},name:${q(m.name)},base_power:${m.basePower},accuracy:${m.accuracy===true?'Accuracy::Always':`Accuracy::Percent(${m.accuracy})`},pp:${m.pp},priority:${m.priority},category:Category::${m.category},move_type:${type(m.type)},target:MoveTarget::${targetNames[targets.indexOf(m.target)]},flags:${flags(m.flags)},crit_ratio:${m.critRatio},no_pp_boosts:${m.noPPBoosts},ignore_immunity:${m.ignoreImmunity},traits:${traits},multihit:[${multi}],recoil:${ratio(m.recoil)},drain:${ratio(m.drain)},damage:${m.damage==='level'?'DamageSpec::Level':typeof m.damage==='number'?`DamageSpec::Fixed(${m.damage})`:'DamageSpec::None'},self_destruct:SelfDestruct::${m.selfdestruct==='always'?'Always':m.selfdestruct==='ifHit'?'IfHit':'None'},offensive_stat:${m.overrideOffensiveStat?`Some(StatId::${statNames[m.overrideOffensiveStat]})`:'None'},defensive_stat:${m.overrideDefensiveStat?`Some(StatId::${statNames[m.overrideDefensiveStat]})`:'None'},offensive_target:${m.overrideOffensivePokemon==='target'},effects:${moveEffects(m,eid)},self_boost:${m.selfBoost?`Some(&${moveEffects(m.selfBoost,eid,'selfBoost')})`:'None'},secondaries:&[${secondaries}]},`;}).join('\n')+'\n];\n';
dataBody+='pub static ABILITIES:&[AbilityData]=&[\n'+objects.abilities.map(e=>`AbilityData{id:${ref('abilities',e.id)},flags:${flags(e.value.flags)},suppress_weather:${!!e.value.suppressWeather}},`).join('\n')+'\n];\n';
dataBody+='pub static ITEMS:&[ItemData]=&[\n'+objects.items.map(e=>`ItemData{id:${ref('items',e.id)},berry:${e.value.isBerry},choice:${!!e.value.isChoice},gem:${e.value.isGem},ignore_klutz:${e.value.ignoreKlutz},fling_power:${e.value.fling?.basePower||0},natural_gift_power:${e.value.naturalGift?.basePower||0},natural_gift_type:${type(e.value.naturalGift?.type)},on_plate:${type(e.value.onPlate)},forced_forme:${ref('species',e.value.forcedForme)},item_users:${refs('species',e.value.itemUser)}},`).join('\n')+'\n];\n';
// Hypothetical species abilities outside the real battle closure have no MaybeTrapPokemon callbacks.
const extraSlots=[...new Set(objects.species.flatMap(e=>Object.values(e.value.abilities)).filter(a=>!ids.abilities.has(id(a))))].sort();
for(const a of extraSlots)if(typeof dex.abilities.get(a).onFoeMaybeTrapPokemon==='function')throw Error(`Scope misses trapping ability ${a}`);
dataBody+=`pub const HYPOTHETICAL_ONLY_ABILITIES:&[&str]=&[${extraSlots.map(q)}];\n`;
dataBody+=`pub const RULE_ORDER:&[EffectId]=&[${[...dex.formats.getRuleTable(dex.formats.get('gen9randomdoublesbattle')).keys()].filter(r=>ids.rules.has(r)).map(r=>ref('rules',r))}];\n`;
dataBody+=`pub const INITIAL_PSEUDO_WEATHER:&[EffectId]=&[${objects.rules.filter(e=>Object.keys(e.value).some(k=>k.startsWith('on')&&!['onBegin','onTeamPreview','onBattleStart','onValidateRule','onValidateTeam','onChangeSet','onValidateSet'].includes(k))).map(e=>ref('rules',e.id))}];\n`;
dataBody+=`pub const CONDITION_RULE_ALIASES:&[(EffectId,EffectId)]=&[${objects.conditions.filter(e=>ids.rules.has(e.id)).map(e=>`(${ref('conditions',e.id)},${ref('rules',e.id)})`)}];\n`;
dataBody+='pub const SPECIES_CONDITION_VIEWS:&[SpeciesConditionView]=&[\n'+objects.species.filter(e=>dex.conditions.getByID(e.id).exists).map(e=>{
 const c=dex.conditions.getByID(e.id),eid=ids.species.get(e.id);
 for(const[k,v]of Object.entries(c))if(typeof v==='function'&&k.startsWith('on')&&e.value[k]?.toString()!==v.toString())throw Error(`Species condition differs: ${e.id}.${k}`);
 const hs=hooks.flatMap((h,i)=>h.effect===eid&&c[h.key]!==undefined?[`HookId(${i})`]:[]);
 return `SpeciesConditionView{species:${ref('species',e.id)},name:${q(c.name)},effect_type:EffectType::${c.effectType},hooks:&[${hs}]},`;
}).join('\n')+'\n];\n';
dataBody+='pub const TYPE_CHART:[[u8;20];20]=[\n'+types.map(def=>'['+types.map(atk=>dex.types.get(def).damageTaken?.[atk]||0).join(',')+'],').join('\n')+'\n];\n';
const immunityKeys=[...new Set(dex.types.all().flatMap(t=>Object.keys(t.damageTaken)).filter(k=>!dex.types.isName(k)))].sort();
eventBody+=`#[derive(Clone,Copy,Debug,PartialEq,Eq)]\n#[repr(u8)]\npub enum ImmunityId{${immunityKeys.map((k,i)=>k[0].toUpperCase()+k.slice(1)+'='+i)}}\n`;
dataBody+=`pub const IMMUNITY_NAMES:[&str;${immunityKeys.length}]=[${immunityKeys.map(q)}];\n`;
dataBody+=`pub const IMMUNITY_CHART:[[u8;${immunityKeys.length}];20]=[\n${types.map(t=>'['+immunityKeys.map(k=>dex.types.get(t).damageTaken?.[k]||0)+'],').join('\n')}\n];\n`;
const natures=[...dex.natures.all()].sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0);
dataBody+=`pub const NATURES:&[NatureData]=&[${natures.map(n=>`NatureData{key:${q(n.id)},plus:${stats.indexOf(n.plus)},minus:${stats.indexOf(n.minus)}}`)}];\n`;
fs.mkdirSync(OUT,{recursive:true});
for(const[file,body]of [['ids_generated.rs',idsBody],['events_generated.rs',eventBody],['hooks_generated.rs',hookBody],['data_generated.rs',dataBody]]){
 const header=`// GENERATED by tools/codegen/gen-dex.mjs from Showdown ${COMMIT}; do not edit\n// Content SHA256: ${hash(body)}\n`;
 fs.writeFileSync(path.join(OUT,file),header+body);
}
console.log(`Generated ${next-1} effects, ${events.size} events, ${hooks.length} hook entries in ${OUT}`);
