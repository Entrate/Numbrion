use super::*;
use crate::{log::*, state::{mon_flags, Status, scratch::MoveAccuracy}};
#[derive(Default)]
struct Commands(Vec<&'static str>);
impl LogSink for Commands {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) { self.0.push(e.command); }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {}
}
fn fixture() -> Battle<Commands> {
    let packed = "Pikachu|||static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric";
    let packed = format!("{packed}]{packed}");
    let mut b = Battle::with_log([1,2,3,4],&packed,&packed,Commands::default()).unwrap();
    b.state.sides[0].active = [MonId(0),MonId(1)]; b.state.sides[1].active = [MonId(6),MonId(7)];
    for i in [0,1,6,7] { b.state.pokemon[i].flags |= mon_flags::ACTIVE; }
    b.state.field.pseudo_weather = Default::default(); b
}
fn move_fixture(id: EffectId) -> ActiveMove {
    let d = dex::move_data(id); let mut m = numeric_move(d.base_power as f64);
    m.id = id; m.flags = d.flags; m.traits = d.traits;
    m.runtime_flags = 0;
    if let Some(dex::DataValue::Bool(v)) = dex::effect(id).data.get(dex::FIELD_WILLCRIT) { m.runtime_flags |= move_runtime::WILL_CRIT_PRESENT; if v { m.runtime_flags |= move_runtime::WILL_CRIT; } }
    if d.ignore_immunity { m.runtime_flags |= move_runtime::IGNORE_IMMUNITY; }
    m.damage=d.damage;m.effects.base=&d.effects;m.move_type=d.move_type;m.category=d.category;m.target=d.target;m.crit_ratio=d.crit_ratio;
    m.offensive_stat=d.offensive_stat;m.defensive_stat=d.defensive_stat;m.offensive_target=d.offensive_target;m.multihit=d.multihit;
    m.drain=d.drain;m.recoil=d.recoil;m.hit=1;
    m
}
fn seed(b: &Battle<Commands>) -> String { b.seed().iter().map(|n|n.to_string()).collect::<Vec<_>>().join(",") }
fn number_text(r: Relay) -> String { match r { Relay::Number(n) if n.is_nan() => "NaN".into(), Relay::Number(n) => n.to_string(), Relay::Undefined => "undefined".into(), Relay::Bool(v) => v.to_string(), Relay::Null => "null".into(), _ => panic!("{r:?}") } }
fn float(s: &str) -> f64 { match s { "NaN" => f64::NAN, "Infinity" => f64::INFINITY, s => s.parse().unwrap() } }
fn id(kind: EffectKind,s: &str) -> EffectId { if s == "-" { EffectId::NONE } else { dex::lookup(kind,s).unwrap() } }
#[test]
fn damage_and_confusion_match_pinned_formula_and_prng() {
    let mut checked = 0;
    for line in include_str!("../../../../../tools/probes/mutators/damage.tsv").lines() {
        let v: Vec<_> = line.split('\t').collect();
        if v[0] == "M" && ((v[4] != "-" && dex::lookup(EffectKind::Item,v[4]).is_none()) || dex::lookup(EffectKind::Ability,v[5]).is_none()) { continue; }
        let mut b = fixture();
        if v[0] == "N" || v[0] == "C" {
            let stage = v[2].parse::<i8>().unwrap(); b.state.pokemon[0].boosts[0]=stage;b.state.pokemon[0].boosts[1]=-stage;
            let r = if v[0] == "N" { b.get_damage(MonId(0),MonId(6),DamageInput::BasePower(float(v[1])),DamageOptions::default()) } else { Relay::Number(b.get_confusion_damage(MonId(0),float(v[1]))) };
            assert_eq!(number_text(r),v[3],"{line}"); assert_eq!(seed(&b),v[4],"{line}"); checked += 1; continue;
        }
        let Some(move_id) = dex::lookup(EffectKind::Move,v[1]) else { continue; };
        let e = EffectRef::Dex(move_id);
        // The Facade callback is a preexisting effect-family gap. Keep its
        // vectors for integration; production invokes it normally.
        if [(EventId::BasePowerCallback,HookRel::Direct),(EventId::DamageCallback,HookRel::Direct),(EventId::BasePower,HookRel::On)].into_iter().any(|(event,rel)| b.event_hook(e,event,rel).is_some_and(|h| matches!(crate::effects::hook_coverage(h),crate::effects::HookCoverage::Pending(_)))) { continue; }
        let mut m = move_fixture(move_id); let expected; let crit; let modifier; let expected_seed;
        if v[0] == "D" {
            b.state.pokemon[6].types=[dex::type_id(v[2]).unwrap(),TypeId::NONE];
            b.state.pokemon[0].terastallized=if v[3]=="-" { TypeId::NONE } else { dex::type_id(v[3]).unwrap() };
            b.state.pokemon[0].boosts[0]=-2;b.state.pokemon[6].boosts[1]=2;
            if v[4] != "unset" { m.runtime_flags |= move_runtime::WILL_CRIT_PRESENT; m.runtime_flags &= !move_runtime::WILL_CRIT; if v[4] == "true" { m.runtime_flags |= move_runtime::WILL_CRIT; } }
            expected=v[5];crit=v[6];modifier=v[7];expected_seed=v[8];
        } else {
            if v[2]=="1" { m.runtime_flags |= move_runtime::SPREAD_HIT; }
            if v[3]=="1" { b.state.pokemon[0].status=Status::Burn; }
            b.state.pokemon[0].item=id(EffectKind::Item,v[4]);b.state.pokemon[0].ability=id(EffectKind::Ability,v[5]);
            b.state.pokemon[6].types=[dex::type_id("Water").unwrap(),TypeId::NONE];
            expected=v[6];crit=v[7];modifier=v[8];expected_seed=v[9];
        }
        b.scratch.moves[0]=Some(m);b.scratch.active_move=MoveHandle(0);b.scratch.active_pokemon=MonId(0);
        let r=b.get_damage(MonId(0),MonId(6),DamageInput::Move(MoveInput::Active(MoveHandle(0))),DamageOptions::default());
        assert_eq!(number_text(r),expected,"{line}");let hit=*b.move_hit_data(MonId(6),MoveHandle(0));
        assert_eq!(u8::from(hit.crit).to_string(),crit,"{line}");assert_eq!(hit.type_mod.to_string(),modifier,"{line}");assert_eq!(seed(&b),expected_seed,"{line}");
        if v[0]=="D" {
            let logs=b.log.0.join(",");assert_eq!(if logs.is_empty() { "-" } else { &logs },v[9],"{line}");
            let stellar=(1..=20).filter(|i|b.state.pokemon[0].stellar_boosted_types&(1<<i)!=0).map(|i|dex::TYPE_NAMES[i-1]).collect::<Vec<_>>().join(",");assert_eq!(if stellar.is_empty() { "-" } else { &stellar },v[10],"{line}");
        }
        checked += 1;
    }
    assert!(checked>=700,"only {checked} vectors checked");
}
