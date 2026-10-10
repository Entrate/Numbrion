use crate::{Battle, dex, event::{EffectRef, EventArg, Relay}, ids::*, log::TextLog, prng::Prng,
    state::{Status, mon_flags}};

#[test]
fn real_healing_callbacks_match_hp_status_logs_and_draws() {
    let packed = "Pikachu|||dancer|thunderbolt|Serious||M|||100|,,,,,Electric";
    for line in include_str!("vectors.tsv").lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<_> = line.split('\t').collect();
        let seed = |text: &str| -> [u16; 4] {
            text.split(',').map(|v| v.parse().unwrap()).collect::<Vec<_>>().try_into().unwrap()
        };
        let id = dex::lookup(match fields[0] {
            "shedskin" | "regenerator" => EffectKind::Ability,
            "leftovers" | "flameorb" | "toxicorb" => EffectKind::Item,
            _ => EffectKind::Move,
        }, fields[0]).unwrap();
        let event = match fields[1] { "Residual" => EventId::Residual, "SwitchOut" => EventId::SwitchOut, _ => EventId::Hit };
        let team = format!("{packed}]{packed}");
        let mut b = Battle::from_players(seed(fields[4]), ("Alice", &team), ("Bob", &team), TextLog::default()).unwrap();
        b.state.sides[0].active = [MonId(0), MonId(1)];
        b.state.sides[1].active = [MonId(6), MonId(7)];
        for i in [0,1,6,7] { b.state.pokemon[i].flags |= mon_flags::ACTIVE; }
        b.state.pokemon[0].hp = fields[2].parse().unwrap();
        b.state.pokemon[0].status = match fields[3] { "par" => Status::Paralysis, "tox" => Status::Toxic, _ => Status::None };
        b.state.prng = Prng::from_seed(seed(fields[4]));
        let target = EventArg::Holder(Holder::mon(MonId(0)));
        let result = b.single_event(event, EffectRef::Dex(id), None, target, target, EffectRef::Dex(id), Relay::Undefined, None);
        let expected_relay = match fields[7] { "true" => Relay::Bool(true), "false" => Relay::Bool(false), "" => Relay::NotFail, "undefined" => Relay::Undefined, v => panic!("unknown relay {v}") };
        assert_eq!(result, expected_relay, "{line}");
        assert_eq!(b.state.pokemon[0].hp, fields[5].parse::<u16>().unwrap(), "{line}");
        let status = match b.state.pokemon[0].status { Status::None => "-", Status::Paralysis => "par", Status::Toxic => "tox", Status::Burn => "brn", other => panic!("unexpected status {other:?}") };
        assert_eq!(status, fields[6], "{line}");
        assert_eq!(b.seed(), seed(fields[8]), "{line}");
        let mut logs = Vec::new(); b.drain_log(&mut logs);
        assert_eq!(logs.join("\\n"), fields[9], "{line}");
    }
}
