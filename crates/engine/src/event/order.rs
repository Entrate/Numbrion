//! Exact selection sort, not Rust sort; sim/battle.ts:408-467.
use super::{HANDLER_CAPACITY, Listener, Priority};
use crate::{
    Battle,
    dex::{self, EffectType, HookRel},
    ids::{EventId, Holder},
    log::LogSink,
    prng::Prng,
    state::{CellId, choices::Action, present},
};
#[derive(Clone, Copy, Debug)]
pub enum SortOrder {
    Priority,
    Redirect,
    LeftToRight,
}
pub trait SpeedSortable: Copy {
    /// Ports battle.ts:408-415. PRNG: none; all speed/priority values are cached.
    fn sort_key(&self) -> Priority;
}
impl SpeedSortable for Listener {
    fn sort_key(&self) -> Priority {
        self.priority
    }
}
impl SpeedSortable for Action {
    fn sort_key(&self) -> Priority {
        Priority {
            order: self.order as f64,
            priority: self.priority, // L already adds fractionalPriority at battle.ts:2651.
            speed: self.speed as f64,
            sub_order: 0.0,
            effect_order: 0,
            index: 0,
            redirect_order: None,
        }
    }
}
/// Ports sim/battle.ts:433-467. PRNG: shuffle each selected tied block (n-1 draws),
/// including ties with identical object keys. Stable sorts use comparator separately.
pub fn speed_sort<T: SpeedSortable>(prng: &mut Prng, list: &mut [T], order: SortOrder) {
    if list.len() < 2 {
        return;
    }
    assert!(
        list.len() <= HANDLER_CAPACITY,
        "speed-sort capacity exceeded"
    );
    // Most lists are short; avoid clearing (and stack-probing) the full-capacity buffer.
    if list.len() <= 16 {
        selection_speed_sort::<T, 16>(prng, list, order);
    } else {
        long_speed_sort(prng, list, order);
    }
}
/// Out of line, so callers' frames do not reserve the full-capacity buffer.
#[inline(never)]
fn long_speed_sort<T: SpeedSortable>(prng: &mut Prng, list: &mut [T], order: SortOrder) {
    selection_speed_sort::<T, HANDLER_CAPACITY>(prng, list, order);
}
fn selection_speed_sort<T: SpeedSortable, const N: usize>(
    prng: &mut Prng,
    list: &mut [T],
    order: SortOrder,
) {
    let mut indexes = [0u16; N];
    let mut sorted = 0;
    while sorted + 1 < list.len() {
        indexes[0] = sorted as u16;
        let mut count = 1;
        for i in sorted + 1..list.len() {
            let delta = compare_priority(
                list[indexes[0] as usize].sort_key(),
                list[i].sort_key(),
                order,
            );
            if delta < 0.0 {
                continue;
            }
            if delta > 0.0 {
                indexes[0] = i as u16;
                count = 1;
            }
            if delta == 0.0 {
                indexes[count] = i as u16;
                count += 1;
            }
        }
        for (i, &index) in indexes[..count].iter().enumerate() {
            let index = index as usize;
            if index != sorted + i {
                list.swap(sorted + i, index);
            }
        }
        if count > 1 {
            prng.shuffle(list, sorted, sorted + count);
        }
        sorted += count;
    }
}
/// Ports sim/battle.ts:408-430. PRNG: none. Return JS comparator sign, preserving
/// falsy zero order, explicit default keys, quarter-speed and redirect holder order.
pub fn compare_priority(a: Priority, b: Priority, order: SortOrder) -> f64 {
    // Evaluated lazily in JS || order; later keys are pure, so skipping them is exact.
    macro_rules! first_truthy {
        ($($value:expr),+) => {{
            $(
                let value = $value;
                // JS || advances past NaN as well as either signed zero. Infinity is truthy.
                if value != 0.0 && !value.is_nan() {
                    return value;
                }
            )+
            0.0
        }};
    }
    match order {
        SortOrder::Priority => first_truthy!(
            js_default(a.order, 4294967296.0) - js_default(b.order, 4294967296.0),
            js_default(b.priority, 0.0) - js_default(a.priority, 0.0),
            js_default(b.speed, 0.0) - js_default(a.speed, 0.0),
            js_default(a.sub_order, 0.0) - js_default(b.sub_order, 0.0),
            a.effect_order as f64 - b.effect_order as f64
        ),
        SortOrder::Redirect => first_truthy!(
            js_default(b.priority, 0.0) - js_default(a.priority, 0.0),
            js_default(b.speed, 0.0) - js_default(a.speed, 0.0),
            match (a.redirect_order, b.redirect_order) {
                (Some(a), Some(b)) => a as f64 - b as f64,
                _ => 0.0,
            }
        ),
        SortOrder::LeftToRight => first_truthy!(
            js_default(a.order, 4294967296.0) - js_default(b.order, 4294967296.0),
            js_default(b.priority, 0.0) - js_default(a.priority, 0.0),
            a.index as f64 - b.index as f64
        ),
    }
}

#[inline]
fn js_default(value: f64, default: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        default
    } else {
        value
    }
}

/// Ports stable Array.sort calls in sim/battle.ts:799-802. PRNG: none.
/// Equal keys retain collection order; use speed_sort only where Showdown does.
pub fn stable_sort<T: SpeedSortable>(list: &mut [T], order: SortOrder) {
    for i in 1..list.len() {
        let value = list[i];
        let key = value.sort_key();
        let mut j = i;
        while j > 0 && compare_priority(list[j - 1].sort_key(), key, order) > 0.0 {
            list[j] = list[j - 1];
            j -= 1;
        }
        list[j] = value;
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/battle.ts:954-1020. PRNG: none; cache speed before handlers mutate it.
    pub fn resolve_priority(&self, listener: Listener) -> Listener {
        let mut listener = listener;
        let selector = listener.selector;
        // The requested key supplies metadata even when SwitchIn dispatches onStart.
        let metadata = self
            .event_hook(listener.effect, selector.event, selector.rel)
            .map(|hook| &dex::HOOKS[hook.0 as usize]);
        listener.priority.order = metadata.map_or(0.0, |hook| js_default(hook.order, 0.0));
        listener.priority.priority = metadata.map_or(0.0, |hook| js_default(hook.priority, 0.0));
        listener.priority.sub_order = metadata.map_or(0.0, |hook| js_default(hook.sub_order, 0.0));

        let effect_type = self.event_effect_type(listener.effect);
        let state = listener.state.map(|reference| {
            let state = &self.state.effects.cells[reference.cell.0 as usize];
            assert_eq!(
                state.generation, reference.generation,
                "stale listener state"
            );
            state
        });
        if listener.priority.sub_order == 0.0 {
            listener.priority.sub_order = match effect_type {
                EffectType::Condition => 2.0,
                EffectType::Weather
                | EffectType::Format
                | EffectType::Rule
                | EffectType::Ruleset => 5.0,
                EffectType::Ability => 7.0,
                EffectType::Item => 8.0,
                _ => 0.0,
            };
            if effect_type == EffectType::Condition {
                if let Some(state) = state {
                    if state.present & present::TARGET != 0 {
                        if (12..14).contains(&state.target.0) {
                            listener.priority.sub_order =
                                if state.present & present::SLOT_CONDITION != 0 {
                                    3.0
                                } else {
                                    4.0
                                };
                        } else if state.target == Holder::FIELD {
                            listener.priority.sub_order = 5.0;
                        }
                    }
                }
            } else if effect_type == EffectType::Ability {
                let effect_id = self.event_effect_id(listener.effect);
                if effect_id == dex::ABILITY_POISONTOUCH {
                    listener.priority.sub_order = 6.0;
                } else if effect_id.0 != 0 {
                    // Perish Body/Stall are outside current scope, but preserve defaults
                    // if the pinned closure later acquires their identifiers.
                    match dex::effect(effect_id).key {
                        "perishbody" => listener.priority.sub_order = 6.0,
                        "stall" => listener.priority.sub_order = 9.0,
                        _ => {}
                    }
                }
            }
        }
        let switch_in = matches!(
            selector.event,
            EventId::BeforeSwitchIn
                | EventId::FieldSwitchIn
                | EventId::SideSwitchIn
                | EventId::SwitchIn
        );
        listener.priority.effect_order = if switch_in || selector.event == EventId::RedirectTarget {
            state.map_or(0, |state| state.effect_order)
        } else {
            0
        };
        listener.priority.speed = 0.0;
        listener.priority.redirect_order = None;
        if listener.holder.0 < 12 {
            let pokemon = &self.state.pokemon[listener.holder.0 as usize];
            listener.priority.speed = pokemon.speed as f64;
            if effect_type == EffectType::Ability
                && self.event_effect_id(listener.effect) == dex::ABILITY_MAGICBOUNCE
                && selector.event == EventId::TryHitSide
                && selector.rel == HookRel::Ally
            {
                // getStat('spe', true, true) makes no callbacks or draws in gen 9.
                listener.priority.speed = pokemon.stored_stats[4].min(10000) as f64;
            }
            if switch_in {
                let field_position = listener.holder.0 / 6 + 2 * pokemon.position;
                let rank = self
                    .state
                    .speed_order
                    .iter()
                    .position(|&position| position == field_position)
                    .map_or(-1, |rank| rank as i32);
                listener.priority.speed -= rank as f64 / 4.0;
            }
            listener.priority.redirect_order = Some(if pokemon.ability_state == CellId::NONE {
                0
            } else {
                self.state.effects.cells[pokemon.ability_state.0 as usize].effect_order
            });
        }
        listener
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event::{CollectMode, EffectRef, EndHandler, HookSelector},
        ids::*,
        log::NoLog,
    };

    #[derive(Clone, Copy, Debug)]
    struct Entry {
        id: u8,
        key: Priority,
    }
    impl SpeedSortable for Entry {
        fn sort_key(&self) -> Priority {
            self.key
        }
    }
    fn entries(speeds: &[u16]) -> Vec<Entry> {
        speeds
            .iter()
            .enumerate()
            .map(|(id, &speed)| Entry {
                id: id as u8,
                key: Priority {
                    speed: speed as f64,
                    ..Priority::default()
                },
            })
            .collect()
    }
    fn ids(list: &[Entry]) -> Vec<u8> {
        list.iter().map(|entry| entry.id).collect()
    }
    fn battle() -> Battle<NoLog> {
        let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
        Battle::new([1, 2, 3, 4], packed, packed).unwrap()
    }
    fn listener(effect: EffectId, event: EventId, rel: HookRel, holder: Holder) -> Listener {
        Listener {
            effect: EffectRef::Dex(effect),
            selector: HookSelector {
                event,
                rel,
                mode: CollectMode::Callback,
            },
            hook: None,
            state: None,
            holder,
            priority: Priority::default(),
            target_index: 0,
            end: EndHandler::None,
        }
    }

    #[test]
    fn selection_swaps_and_tie_draws_match_pinned_showdown() {
        // Pinned Battle.prototype.speedSort + Gen5 PRNG oracle; intermediate
        // selection swaps change which original members reach later tie shuffles.
        let mut list = entries(&[1, 3, 3, 2, 1, 3]);
        let mut prng = Prng::from_seed([1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list, SortOrder::Priority);
        assert_eq!(ids(&list), [2, 5, 1, 3, 0, 4]);
        assert_eq!(prng.seed(), [43514, 9542, 40559, 8561]);

        let mut list = entries(&[0, 4, 3, 4, 1, 3, 2, 0]);
        let mut prng = Prng::from_seed([1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list, SortOrder::Priority);
        assert_eq!(ids(&list), [1, 3, 5, 2, 6, 4, 7, 0]);
        assert_eq!(prng.seed(), [43514, 9542, 40559, 8561]);
    }

    #[test]
    fn short_and_long_buffers_sort_identically_around_the_threshold() {
        let keys = [3.0, 1.0, 3.0, f64::NAN, 2.0, 0.0, 3.0, f64::INFINITY, 1.0, 2.0];
        for len in 2..=20 {
            for seed in 0..8u16 {
                let mut list: Vec<Entry> = (0..len)
                    .map(|i| Entry {
                        id: i as u8,
                        key: Priority {
                            speed: keys[(i * 7 + seed as usize) % keys.len()],
                            sub_order: (i % 3) as f64,
                            ..Priority::default()
                        },
                    })
                    .collect();
                let mut long = list.clone();
                let mut prng = Prng::from_seed([seed, 2, 3, 4]);
                let mut long_prng = Prng::from_seed([seed, 2, 3, 4]);
                speed_sort(&mut prng, &mut list, SortOrder::Priority);
                selection_speed_sort::<_, HANDLER_CAPACITY>(
                    &mut long_prng,
                    &mut long,
                    SortOrder::Priority,
                );
                assert_eq!(ids(&list), ids(&long), "len {len} seed {seed}");
                assert_eq!(prng.seed(), long_prng.seed(), "len {len} seed {seed}");
            }
        }
    }

    #[test]
    fn all_tied_group_matches_oracle_and_unique_keys_do_not_draw() {
        let mut list = entries(&[100; 8]);
        let mut prng = Prng::from_seed([1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list, SortOrder::Priority);
        assert_eq!(ids(&list), [3, 7, 5, 0, 1, 6, 4, 2]);
        assert_eq!(prng.seed(), [62891, 40560, 22227, 62965]);
        let mut list = entries(&[1, 4, 2, 3]);
        let mut prng = Prng::from_seed([1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list, SortOrder::Priority);
        assert_eq!(ids(&list), [1, 3, 2, 0]);
        assert_eq!(prng.seed(), [1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list[..0], SortOrder::Priority);
        speed_sort(&mut prng, &mut list[..1], SortOrder::Priority);
        assert_eq!(prng.seed(), [1, 2, 3, 4]);
    }

    #[test]
    fn mixed_orders_and_subkeys_match_oracle() {
        let mut list = entries(&[0, 0, 3, 0, 3, 2, 0]);
        for (entry, (order, priority)) in list.iter_mut().zip([
            (0., 1.),
            (2., 2.),
            (1., 0.),
            (2., 2.),
            (1., 0.),
            (1., 0.),
            (0., 1.),
        ]) {
            entry.key.order = order;
            entry.key.priority = priority;
        }
        let mut prng = Prng::from_seed([1, 2, 3, 4]);
        speed_sort(&mut prng, &mut list, SortOrder::Priority);
        assert_eq!(ids(&list), [2, 4, 5, 1, 3, 6, 0]);
        assert_eq!(prng.seed(), [43514, 9542, 40559, 8561]);
        let mut a = Priority::default();
        let mut b = a;
        a.order = 0.;
        b.order = 1.;
        assert!(compare_priority(a, b, SortOrder::Priority) > 0.);
        a.order = f64::NAN;
        assert!(compare_priority(a, b, SortOrder::Priority) > 0.);
        a.order = f64::INFINITY;
        b.order = f64::INFINITY;
        a.sub_order = 2.;
        b.sub_order = 3.;
        assert_eq!(compare_priority(a, b, SortOrder::Priority), -1.);
        a.sub_order = 0.;
        b.sub_order = 0.;
        a.effect_order = 1;
        b.effect_order = 2;
        assert_eq!(compare_priority(a, b, SortOrder::Priority), -1.);
    }

    #[test]
    fn redirect_and_left_sort_are_stable_and_use_different_keys() {
        let mut list = entries(&[50; 4]);
        for (entry, (index, redirect)) in
            list.iter_mut()
                .zip([(2, Some(7)), (0, Some(7)), (0, Some(3)), (1, None)])
        {
            entry.key.index = index;
            entry.key.redirect_order = redirect;
        }
        let mut redirected = list.clone();
        stable_sort(&mut redirected, SortOrder::Redirect);
        assert_eq!(ids(&redirected), [2, 0, 1, 3]);
        stable_sort(&mut list, SortOrder::LeftToRight);
        assert_eq!(ids(&list), [1, 2, 3, 0]);
    }

    #[test]
    fn resolve_priority_uses_requested_key_and_holder_context() {
        let mut battle = battle();
        let mon = MonId(0);
        battle.state.pokemon[0].speed = 200;
        battle.state.pokemon[0].stored_stats[4] = 123;
        battle.state.speed_order = [1, 2, 0, 3];
        let raw = listener(
            dex::ABILITY_MAGICBOUNCE,
            EventId::TryHitSide,
            HookRel::Ally,
            Holder::mon(mon),
        );
        assert_eq!(battle.resolve_priority(raw).priority.speed, 123.);

        let cell = battle.state.pokemon[0].ability_state;
        battle.state.effects.cells[cell.0 as usize].effect_order = 17;
        let mut switched = listener(
            dex::ABILITY_INTIMIDATE,
            EventId::SwitchIn,
            HookRel::On,
            Holder::mon(mon),
        );
        switched.hook = battle.event_hook(switched.effect, EventId::Start, HookRel::On);
        switched.state = Some(battle.state.effects.capture(cell));
        let priority = battle.resolve_priority(switched).priority;
        assert_eq!(priority.speed, 199.5);
        assert_eq!(priority.effect_order, 17);
        assert_eq!(priority.redirect_order, Some(17));

        let poison = listener(
            dex::ABILITY_POISONTOUCH,
            EventId::DamagingHit,
            HookRel::Source,
            Holder::mon(mon),
        );
        assert_eq!(battle.resolve_priority(poison).priority.sub_order, 6.);
        let side = Holder::side(SideId(0));
        let cell = battle
            .state
            .effects
            .alloc(side, side, dex::CONDITION_REFLECT, 8);
        let mut reflect = listener(dex::CONDITION_REFLECT, EventId::Residual, HookRel::On, side);
        reflect.state = Some(battle.state.effects.capture(cell));
        assert_eq!(battle.resolve_priority(reflect).priority.sub_order, 4.);
        battle.state.effects.cells[cell.0 as usize].present |= present::SLOT_CONDITION;
        assert_eq!(battle.resolve_priority(reflect).priority.sub_order, 3.);
        battle.state.effects.cells[cell.0 as usize].target = Holder::FIELD;
        assert_eq!(battle.resolve_priority(reflect).priority.sub_order, 5.);
        battle.state.effects.cells[cell.0 as usize].present &= !present::TARGET;
        assert_eq!(battle.resolve_priority(reflect).priority.sub_order, 2.);
    }
}
