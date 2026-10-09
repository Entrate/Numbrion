//! Event execution ports of sim/battle.ts:469-655,762-944. Discovery never draws;
//! speedSort draws per tied block, and callbacks own every additional PRNG draw.
use crate::{
    Battle,
    actions::TargetResults,
    dex::{self, HookId, HookRel, HookValue},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::{CellId, CellRef, EffectCell, mon_flags, present},
};
impl SpeedSortable for Option<Listener> {
    fn sort_key(&self) -> Priority {
        self.expect("empty captured listener").priority
    }
}
impl<L: LogSink> Battle<L> {
    fn reserve_handlers(&mut self) -> u8 {
        let i = self.scratch.handler_depth;
        assert!((i as usize) < CALL_DEPTH, "handler stack overflow");
        assert_eq!(self.scratch.handlers[i as usize].len, 0);
        self.scratch.handler_depth += 1;
        i
    }
    fn finish_handlers(&mut self, buffer: u8) {
        self.release_handlers(buffer);
        assert_eq!(self.scratch.handler_depth, buffer + 1);
        self.scratch.handler_depth -= 1;
    }
    fn sort_handlers(&mut self, buffer: u8, event: EventId, fast: bool) {
        let b = &mut self.scratch.handlers[buffer as usize];
        let entries = &mut b.entries[..b.len as usize];
        if matches!(
            event,
            EventId::Invulnerability
                | EventId::TryHit
                | EventId::DamagingHit
                | EventId::EntryHazard
        ) {
            order::stable_sort(entries, SortOrder::LeftToRight);
        } else if fast {
            order::stable_sort(entries, SortOrder::Redirect);
        } else {
            speed_sort(&mut self.state.prng, entries, SortOrder::Priority);
        }
    }
    fn push_frame(
        &mut self,
        event: EventId,
        target: EventArg,
        source: EventArg,
        effect: EffectRef,
        relay: Relay,
        has_relay: bool,
        single: bool,
        fast: bool,
    ) -> u8 {
        let i = self.scratch.event_depth;
        self.scratch.frames[i as usize] = Some(EventFrame {
            event,
            target,
            source,
            source_effect: effect,
            relay,
            modifier: if single { 0 } else { 4096 },
            modifier_present: !single,
            has_relay,
            fast_exit: fast,
            args: [EventArg::Undefined; 4],
            arg_count: 0,
        });
        self.scratch.event_depth += 1;
        self.scratch.current_frame = i;
        self.set_args(i, target, source, effect, relay, has_relay);
        i
    }
    fn set_args(
        &mut self,
        i: u8,
        target: EventArg,
        source: EventArg,
        effect: EffectRef,
        relay: Relay,
        has: bool,
    ) {
        let f = self.scratch.frames[i as usize].as_mut().unwrap();
        f.target = target;
        f.relay = relay;
        let n = usize::from(has);
        f.args = [EventArg::Undefined; 4];
        if has {
            f.args[0] = EventArg::Relay(relay);
        }
        f.args[n] = target;
        f.args[n + 1] = source;
        f.args[n + 2] = EventArg::Effect(effect);
        f.arg_count = (n + 3) as u8;
    }
    fn pop_frame(&mut self, i: u8, parent: u8) {
        assert_eq!(self.scratch.event_depth, i + 1);
        self.scratch.frames[i as usize] = None;
        self.scratch.event_depth -= 1;
        self.scratch.current_frame = parent;
    }
    pub(crate) fn execute_hook(&mut self, hook: HookId, cx: HookCtx) -> Relay {
        crate::effects::dispatch_hook(hook, self, cx)
    }
    /// Borrow-free callback invocation: captured cells are pinned through nested removal.
    fn invoke(
        &mut self,
        hook: HookId,
        effect: EffectRef,
        state: Option<CellRef>,
        holder: Holder,
        index: u8,
        set_target: bool,
    ) -> Relay {
        let temporary = state.is_none();
        let cell = state.map_or_else(
            || {
                self.state
                    .effects
                    .alloc(Holder::BATTLE, Holder::NONE, EffectId::NONE, 0)
            },
            |r| r.cell,
        );
        let captured = self.state.effects.pin(cell);
        if let Some(state) = state {
            assert_eq!(state, captured, "stale effect state");
        }
        if set_target {
            let c = &mut self.state.effects.cells[cell.0 as usize];
            c.target = holder;
            c.present |= present::TARGET;
        }
        let saved = (
            self.scratch.current_effect,
            self.scratch.current_state,
            self.scratch.current_context,
        );
        self.scratch.current_effect = effect;
        self.scratch.current_state = Some(captured);
        let cx = self.context_for(captured, holder, index);
        self.scratch.current_context = Some(cx);
        let result = self.execute_hook(hook, cx);
        (
            self.scratch.current_effect,
            self.scratch.current_state,
            self.scratch.current_context,
        ) = saved;
        self.state.effects.unpin(captured);
        if temporary {
            self.state.effects.release(cell);
        }
        result
    }
    fn context_for(&self, state: CellRef, holder: Holder, index: u8) -> HookCtx {
        let f = self
            .scratch
            .frames
            .get(self.scratch.current_frame as usize)
            .and_then(|f| *f);
        HookCtx {
            frame: self.scratch.current_frame,
            call_args: 255,
            active_move: self.scratch.active_move.0,
            state,
            holder,
            target: f.map_or(EventArg::Undefined, |f| f.target),
            source: f.map_or(EventArg::Undefined, |f| f.source),
            source_effect: f.map_or(EffectRef::None, |f| f.source_effect),
            relay: f.map_or(Relay::Undefined, |f| f.relay),
            target_index: index,
        }
    }
    /// battle.ts:575-655. Only undefined supplies the default true; null is a real relay.
    pub fn single_event(
        &mut self,
        event: EventId,
        effect: EffectRef,
        state: Option<CellRef>,
        target: EventArg,
        source: EventArg,
        source_effect: EffectRef,
        relay: Relay,
        custom_hook: Option<HookId>,
    ) -> Relay {
        self.check_event_limits(event, true);
        let has = relay != Relay::Undefined;
        let relay = if has { relay } else { Relay::Bool(true) };
        if self.single_event_suppressed(event, effect, target) {
            return relay;
        }
        let hook = custom_hook
            .filter(|h| match dex::HOOKS[h.0 as usize].value {
                HookValue::Absent => false,
                HookValue::Constant(v) => constant_truthy(v),
                HookValue::Function => true,
            })
            .or_else(|| {
                self.event_hook(effect, event, HookRel::On)
                    .filter(|h| !matches!(dex::HOOKS[h.0 as usize].value, HookValue::Absent))
            });
        let Some(hook) = hook else {
            return relay;
        };
        let parent = self.scratch.current_frame;
        let i = self.push_frame(
            event,
            target,
            source,
            source_effect,
            relay,
            has,
            true,
            false,
        );
        let result = self.invoke(hook, effect, state, Self::arg_holder(target), 255, false);
        self.pop_frame(i, parent);
        if result == Relay::Undefined {
            relay
        } else {
            result
        }
    }
    /// battle.ts:762-944. Scalar null/undefined omit relay arguments and default to true.
    pub fn run_event(
        &mut self,
        event: EventId,
        target: EventArg,
        source: EventArg,
        source_effect: EffectRef,
        relay: Relay,
        options: RunEventOptions,
    ) -> Relay {
        match self.run_inner(
            event,
            EventTarget::Single(target),
            source,
            source_effect,
            relay,
            TargetResults::default(),
            options,
        ) {
            EventResult::Single(r) => r,
            _ => unreachable!(),
        }
    }
    /// battle.ts:762-944. relays.len==0 means omitted; otherwise an explicit JS relay array.
    pub fn run_event_spread(
        &mut self,
        event: EventId,
        targets: EventTargets,
        source: EventArg,
        source_effect: EffectRef,
        relays: TargetResults,
        options: RunEventOptions,
    ) -> TargetResults {
        match self.run_inner(
            event,
            EventTarget::Spread(targets),
            source,
            source_effect,
            Relay::Undefined,
            relays,
            options,
        ) {
            EventResult::Spread(r) => r,
            _ => unreachable!(),
        }
    }
    fn run_inner(
        &mut self,
        event: EventId,
        mut target: EventTarget,
        source: EventArg,
        effect: EffectRef,
        mut relay: Relay,
        mut relays: TargetResults,
        options: RunEventOptions,
    ) -> EventResult {
        self.check_event_limits(event, false);
        if let EventTarget::Single(t) = target {
            if !arg_truthy(t) {
                target = EventTarget::Single(EventArg::Holder(Holder::BATTLE));
            }
        }
        let selector = HookSelector {
            event,
            rel: HookRel::On,
            mode: CollectMode::Callback,
        };
        let b = self.reserve_handlers();
        self.find_event_handlers(target, selector, Self::arg_mon(source), b);
        let mut extra = None;
        if options.on_effect {
            assert_ne!(effect, EffectRef::None, "onEffect passed without an effect");
            if let Some(h) = self
                .event_hook(effect, event, HookRel::On)
                .filter(|h| !matches!(dex::HOOKS[h.0 as usize].value, HookValue::Absent))
            {
                let EventTarget::Single(t) = target else {
                    panic!("onEffect with array target");
                };
                let c = self
                    .state
                    .effects
                    .alloc(Holder::BATTLE, Holder::NONE, EffectId::NONE, 0);
                extra = Some(c);
                let listener = self.resolve_priority(Listener {
                    effect,
                    selector,
                    hook: Some(h),
                    state: Some(self.state.effects.capture(c)),
                    holder: Self::arg_holder(t),
                    priority: Priority::default(),
                    target_index: 255,
                    end: EndHandler::None,
                });
                self.push_listener(b, listener);
                let buffer = &mut self.scratch.handlers[b as usize];
                buffer.entries[..buffer.len as usize].rotate_right(1);
            }
        }
        self.sort_handlers(b, event, options.fast_exit);
        let spread = matches!(target, EventTarget::Spread(_));
        let has = if spread {
            relays.len != 0
        } else {
            !matches!(relay, Relay::Undefined | Relay::Null)
        };
        let target_arg = match target {
            EventTarget::Single(t) => t,
            EventTarget::Spread(ts) => {
                assert!(ts.len <= 4);
                if relays.len == 0 {
                    relays = TargetResults {
                        values: [Relay::Bool(true); 4],
                        len: ts.len,
                    };
                } else {
                    assert_eq!(relays.len, ts.len);
                }
                EventArg::Undefined
            }
        };
        if !has || spread {
            relay = Relay::Bool(true);
        }
        let parent = self.scratch.current_frame;
        let frame = self.push_frame(
            event,
            target_arg,
            source,
            effect,
            relay,
            has,
            false,
            options.fast_exit,
        );
        let count = self.scratch.handlers[b as usize].len;
        for j in 0..count as usize {
            let h = self.scratch.handlers[b as usize].entries[j].unwrap();
            let mut t = target_arg;
            let mut arg_relay = relay;
            if let EventTarget::Spread(ts) = target {
                let k = h.target_index as usize;
                arg_relay = relays.values[k];
                if !arg_relay.truthy()
                    && !(arg_relay == Relay::Number(0.0) && event == EventId::DamagingHit)
                {
                    continue;
                }
                t = EventArg::Holder(Holder::mon(ts.mons[k]));
            }
            self.set_args(frame, t, source, effect, arg_relay, has);
            if self.run_event_suppressed(event, h) {
                continue;
            }
            let result = if let Some(hook) = h.hook {
                if matches!(dex::HOOKS[hook.0 as usize].value, HookValue::Function) {
                    self.invoke(hook, h.effect, h.state, h.holder, h.target_index, true)
                } else {
                    self.execute_hook(hook, self.current_hook_context())
                }
            } else {
                Relay::Undefined
            };
            if result != Relay::Undefined {
                relay = result;
                if !result.truthy() || options.fast_exit {
                    if spread {
                        relays.values[h.target_index as usize] = result;
                        if relays.values[..relays.len as usize]
                            .iter()
                            .all(|v| !v.truthy())
                        {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        if let Relay::Number(n) = relay {
            if n >= 0.0 && n == n.floor() {
                let modifier = self.scratch.frames[frame as usize].unwrap().modifier;
                relay = Relay::Number(self.modify(n, modifier as f64, 4096.0));
            }
        }
        self.pop_frame(frame, parent);
        self.finish_handlers(b);
        if let Some(c) = extra {
            self.state.effects.release(c);
        }
        if spread {
            EventResult::Spread(relays)
        } else {
            EventResult::Single(relay)
        }
    }
    pub fn priority_event(
        &mut self,
        event: EventId,
        target: EventArg,
        source: EventArg,
        effect: EffectRef,
        relay: Relay,
        on_effect: bool,
    ) -> Relay {
        self.run_event(
            event,
            target,
            source,
            effect,
            relay,
            RunEventOptions {
                on_effect,
                fast_exit: true,
            },
        )
    }
    /// battle.ts:469-483. Only living active cached speeds order this outer pass.
    pub fn each_event(&mut self, event: EventId, effect: EffectRef, relay: Relay) {
        #[derive(Clone, Copy)]
        struct Active {
            m: MonId,
            speed: u16,
        }
        impl SpeedSortable for Active {
            fn sort_key(&self) -> Priority {
                Priority {
                    speed: self.speed as f64,
                    ..Priority::default()
                }
            }
        }
        let mut mons = [Active {
            m: MonId::NONE,
            speed: 0,
        }; 4];
        let mut len = 0;
        for side in [SideId(0), SideId(1)] {
            let (ms, n) = self.living_actives(side);
            for m in ms[..n].iter().copied() {
                mons[len] = Active {
                    m,
                    speed: self.state.pokemon[m.0 as usize].speed,
                };
                len += 1;
            }
        }
        speed_sort(&mut self.state.prng, &mut mons[..len], SortOrder::Priority);
        let effect = if effect == EffectRef::None {
            self.scratch.current_effect
        } else {
            effect
        };
        for p in mons[..len].iter() {
            self.run_event(
                event,
                EventArg::Holder(Holder::mon(p.m)),
                EventArg::Null,
                effect,
                relay,
                RunEventOptions::default(),
            );
        }
        if event == EventId::Weather {
            self.each_event(EventId::Update, effect, relay);
        }
    }
    /// battle.ts:488-568. Expiry precedes state-location validation; slot cells survive fainted holders.
    pub fn field_event(&mut self, event: EventId, targets: Option<EventTargets>) {
        let b = self.reserve_handlers();
        let mode = if event == EventId::Residual {
            CollectMode::Duration
        } else {
            CollectMode::Callback
        };
        if let Some(e) = suffix_event(event, true) {
            self.find_field_event_handlers(
                HookSelector {
                    event: e,
                    rel: HookRel::On,
                    mode,
                },
                None,
                b,
            );
        }
        for side in [SideId(0), SideId(1)] {
            if let Some(e) = suffix_event(event, false) {
                self.find_side_event_handlers(
                    side,
                    HookSelector {
                        event: e,
                        rel: HookRel::On,
                        mode,
                    },
                    None,
                    b,
                );
            }
            for m in self.state.sides[side.0 as usize].active {
                if m == MonId::NONE {
                    continue;
                }
                if event == EventId::SwitchIn {
                    self.find_pokemon_event_handlers(
                        m,
                        HookSelector {
                            event,
                            rel: HookRel::Any,
                            mode: CollectMode::Callback,
                        },
                        b,
                    );
                }
                if targets.is_some_and(|ts| !ts.mons[..ts.len as usize].contains(&m)) {
                    continue;
                }
                let selector = HookSelector {
                    event,
                    rel: HookRel::On,
                    mode,
                };
                self.find_pokemon_event_handlers(m, selector, b);
                let selector = HookSelector {
                    mode: CollectMode::Callback,
                    ..selector
                };
                self.find_side_event_handlers(side, selector, Some(m), b);
                self.find_field_event_handlers(selector, Some(m), b);
                self.find_battle_event_handlers(HookSelector { mode, ..selector }, Some(m), b);
            }
        }
        {
            let buffer = &mut self.scratch.handlers[b as usize];
            speed_sort(
                &mut self.state.prng,
                &mut buffer.entries[..buffer.len as usize],
                SortOrder::Priority,
            );
        }
        let count = self.scratch.handlers[b as usize].len;
        for j in 0..count as usize {
            let h = self.scratch.handlers[b as usize].entries[j].unwrap();
            if h.holder.0 < 12
                && self.state.pokemon[h.holder.0 as usize].flags & mon_flags::FAINTED != 0
                && !h.state.is_some_and(|r| {
                    self.state.effects.cells[r.cell.0 as usize].present & present::SLOT_CONDITION
                        != 0
                })
            {
                continue;
            }
            if event == EventId::Residual && !matches!(h.end, EndHandler::None) {
                if let Some(r) = h.state {
                    let c = &mut self.state.effects.cells[r.cell.0 as usize];
                    if c.duration != -1 && c.duration != 0 {
                        c.duration -= 1;
                        if c.duration == 0 {
                            self.end_listener(h.end);
                            if self.state.ended {
                                break;
                            }
                            continue;
                        }
                    }
                }
            }
            if !self.listener_in_location(h) {
                continue;
            }
            if let Some(hook) = h.hook {
                if hook_truthy(hook) {
                    let e = if h.holder.0 == 14 {
                        suffix_event(event, true)
                    } else if h.holder.0 == 12 || h.holder.0 == 13 {
                        suffix_event(event, false)
                    } else {
                        Some(event)
                    };
                    if let Some(e) = e {
                        self.single_event(
                            e,
                            h.effect,
                            h.state,
                            EventArg::Holder(h.holder),
                            EventArg::Null,
                            EffectRef::None,
                            Relay::Undefined,
                            Some(hook),
                        );
                    }
                }
            }
            self.faint_messages(false, false, true);
            if self.state.ended {
                break;
            }
        }
        self.finish_handlers(b);
    }
    fn end_listener(&mut self, end: EndHandler) {
        match end {
            EndHandler::None => {}
            EndHandler::Status(m) => {
                self.clear_status(m);
            }
            EndHandler::Ability(m) => {
                self.clear_ability(m);
            }
            EndHandler::Item(m) => {
                self.clear_item(m);
            }
            EndHandler::Volatile(m, id) => {
                self.remove_volatile(m, id);
            }
            EndHandler::Side(s, id) => {
                self.remove_side_condition(s, id);
            }
            EndHandler::Slot(s, id) => {
                self.remove_slot_condition(s, id);
            }
            EndHandler::Weather => {
                self.clear_weather();
            }
            EndHandler::Terrain => {
                self.clear_terrain();
            }
            EndHandler::PseudoWeather(id) => {
                self.remove_pseudo_weather(id);
            }
        }
    }
    fn listener_in_location(&self, h: Listener) -> bool {
        let Some(r) = h.state else {
            return true;
        };
        let c = &self.state.effects.cells[r.cell.0 as usize];
        if c.present & present::TARGET == 0 {
            return true;
        }
        let ty = self.event_effect_type(h.effect);
        let expected = match c.target.0 {
            0..=11 => {
                let p = &self.state.pokemon[c.target.0 as usize];
                match ty {
                    dex::EffectType::Ability if c.present & present::PREFIXED_ID == 0 => {
                        Some(p.ability_state)
                    }
                    dex::EffectType::Item if c.present & present::PREFIXED_ID == 0 => {
                        Some(p.item_state)
                    }
                    dex::EffectType::Status => Some(p.status_state),
                    _ => p
                        .volatiles
                        .as_slice()
                        .iter()
                        .copied()
                        .find(|i| self.state.effects.cells[i.0 as usize].id == c.id),
                }
            }
            12..=13 if c.present & present::SLOT_CONDITION == 0 => self.state.sides
                [(c.target.0 - 12) as usize]
                .conditions
                .as_slice()
                .iter()
                .copied()
                .find(|i| self.state.effects.cells[i.0 as usize].id == c.id),
            14 => match ty {
                dex::EffectType::Weather => Some(self.state.field.weather),
                dex::EffectType::Terrain => Some(self.state.field.terrain),
                _ => self
                    .state
                    .field
                    .pseudo_weather
                    .as_slice()
                    .iter()
                    .copied()
                    .find(|i| self.state.effects.cells[i.0 as usize].id == c.id),
            },
            _ => return true,
        };
        expected == Some(r.cell)
            && self.state.effects.cells[r.cell.0 as usize].generation == r.generation
    }
    /// Direct .call preserves owner/state/frame and depth; arguments get separate fixed scratch.
    pub fn call_hook(&mut self, hook: HookId, args: CallArgs) -> Relay {
        assert!(args.len <= 4);
        let i = self.scratch.call_depth;
        assert!((i as usize) < CALL_DEPTH, "direct callback stack overflow");
        self.scratch.calls[i as usize] = Some(args);
        self.scratch.call_depth += 1;
        let saved = self.scratch.current_context;
        let temporary = self.scratch.current_state.is_none();
        let cell = if let Some(r) = self.scratch.current_state {
            r.cell
        } else {
            self.state
                .effects
                .alloc(Holder::BATTLE, Holder::NONE, EffectId::NONE, 0)
        };
        let r = self.state.effects.pin(cell);
        let mut cx = self.current_hook_context();
        cx.state = r;
        cx.call_args = i;
        self.scratch.current_context = Some(cx);
        let result = self.execute_hook(hook, cx);
        self.scratch.current_context = saved;
        self.state.effects.unpin(r);
        if temporary {
            self.state.effects.release(cell);
        }
        self.scratch.call_depth -= 1;
        self.scratch.calls[i as usize] = None;
        result
    }
    pub fn event_arg(&self, cx: HookCtx, index: usize) -> EventArg {
        if cx.call_args != 255 {
            let args = self.scratch.calls[cx.call_args as usize].expect("released direct args");
            if index < args.len as usize {
                args.values[index]
            } else {
                EventArg::Undefined
            }
        } else if let Some(f) = self.event_frame(cx) {
            if index < f.arg_count as usize {
                f.args[index]
            } else {
                EventArg::Undefined
            }
        } else {
            EventArg::Undefined
        }
    }
    pub fn hook_state(&self, cx: HookCtx) -> &EffectCell {
        let c = &self.state.effects.cells[cx.state.cell.0 as usize];
        assert_eq!(c.generation, cx.state.generation, "recycled hook state");
        c
    }
    pub fn hook_state_mut(&mut self, cx: HookCtx) -> &mut EffectCell {
        let c = &mut self.state.effects.cells[cx.state.cell.0 as usize];
        assert_eq!(c.generation, cx.state.generation, "recycled hook state");
        c
    }
    pub fn event_frame(&self, cx: HookCtx) -> Option<&EventFrame> {
        self.scratch
            .frames
            .get(cx.frame as usize)
            .and_then(Option::as_ref)
    }
    pub fn current_hook_context(&self) -> HookCtx {
        self.scratch.current_context.unwrap_or_else(|| {
            self.context_for(
                self.scratch.current_state.unwrap_or(CellRef {
                    cell: CellId::NONE,
                    generation: 0,
                }),
                Holder::NONE,
                255,
            )
        })
    }
    /// battle.ts:582-592,772-775. Diagnostics themselves count toward the log limit.
    pub fn check_event_limits(&mut self, event: EventId, single: bool) {
        let depth = self.scratch.event_depth >= 8;
        let lines = single && self.scratch.unsent_lines > 1000;
        if !depth && !lines {
            return;
        }
        let parent = self
            .scratch
            .frames
            .get(self.scratch.current_frame as usize)
            .and_then(|f| *f)
            .map_or("", |f| dex::EVENT_NAMES[f.event as usize]);
        for args in [
            [LogArg::Text(if depth {
                "STACK LIMIT EXCEEDED"
            } else {
                "LINE LIMIT EXCEEDED"
            })],
            [LogArg::Text("PLEASE REPORT IN BUG THREAD")],
        ] {
            self.add(LogEntry::new("message", &args, &[]));
        }
        self.add(LogEntry::new(
            "message",
            &[LogArg::Parts(&[
                LogArg::Text("Event: "),
                LogArg::Text(dex::EVENT_NAMES[event as usize]),
            ])],
            &[],
        ));
        self.add(LogEntry::new(
            "message",
            &[LogArg::Parts(&[
                LogArg::Text("Parent event: "),
                LogArg::Text(parent),
            ])],
            &[],
        ));
        panic!(
            "{}",
            if depth {
                "Stack overflow"
            } else {
                "Infinite loop"
            }
        );
    }
}
fn arg_truthy(arg: EventArg) -> bool {
    match arg {
        EventArg::Undefined | EventArg::Null | EventArg::Bool(false) => false,
        EventArg::Relay(r) => r.truthy(),
        EventArg::Number(n) => n != 0.0 && !n.is_nan(),
        _ => true,
    }
}
fn constant_truthy(v: dex::DataValue) -> bool {
    match v {
        dex::DataValue::Null | dex::DataValue::Bool(false) => false,
        dex::DataValue::Number(n) => n != 0.0 && !n.is_nan(),
        dex::DataValue::Text(s) => !s.is_empty(),
        _ => true,
    }
}
fn hook_truthy(h: HookId) -> bool {
    match dex::HOOKS[h.0 as usize].value {
        HookValue::Absent => false,
        HookValue::Function => true,
        HookValue::Constant(v) => constant_truthy(v),
    }
}
fn suffix_event(e: EventId, field: bool) -> Option<EventId> {
    use EventId::*;
    Some(match (e, field) {
        (Residual, true) => FieldResidual,
        (Residual, false) => SideResidual,
        (Start, true) => FieldStart,
        (Start, false) => SideStart,
        (End, true) => FieldEnd,
        (End, false) => SideEnd,
        (Restart, true) => FieldRestart,
        (Restart, false) => SideRestart,
        (SwitchIn, true) => FieldSwitchIn,
        (SwitchIn, false) => SideSwitchIn,
        _ => return None,
    })
}
