//! Replay of real Showdown battle logs.
//!
//! Every raw log entry of a recorded battle is translated back into the structured
//! `LogEntry`/`LogArg`/`LogTag` values the engine's call sites would build (idents resolved
//! against a battle state that this test tracks from the log itself), pushed through
//! `Battle::add` / `add_move` / `attr_last_move`, and the resulting `TextLog` must equal the real
//! log byte for byte. HP, details, ident, effect names, tags, split triples and the deferred
//! `attrLastMove` edits are therefore all checked against thousands of real lines.
//!
//! `replay-vectors.tsv` holds a greedy set cover of every line shape found in sample-50,
//! directed-smoke and fuzz-2000 (`tools/probes/textlog/extract-replay.mjs`). The ignored test
//! `replay_corpus_from_env` replays any extracted file (e.g. the whole fuzz corpus).
use super::*;
use std::collections::HashMap;

const VECTORS: &str = include_str!("replay-vectors.tsv");

/// Owned mirror of `LogArg`.
#[derive(Clone, Debug)]
enum A {
    Text(String),
    Empty,
    Mon(MonId),
    Side(u8),
    SideId(u8),
    Player(u8),
    Num(i32),
    Health(MonId),
    Full(MonId),
    Details(MonId),
    Spread(Vec<MonId>),
}

/// Owned mirror of `LogTag`.
#[derive(Clone, Debug)]
enum T {
    From(EffectRef),
    Of(MonId),
    Bare(&'static str),
    Val(&'static str, A),
}

#[derive(Clone, Debug)]
enum Edit {
    Still,
    Miss,
    NoTarget,
    Spread(Vec<MonId>),
}

fn lower_a(a: &A) -> LogArg<'_> {
    match a {
        A::Text(s) => LogArg::Text(s),
        A::Empty => LogArg::Empty,
        A::Mon(m) => LogArg::Mon(*m),
        A::Side(s) => LogArg::Side(SideId(*s)),
        A::SideId(s) => LogArg::SideId(SideId(*s)),
        A::Player(s) => LogArg::PlayerName(SideId(*s)),
        A::Num(n) => LogArg::Number(*n),
        A::Health(m) => LogArg::Health(*m),
        A::Full(m) => LogArg::FullDetails(*m),
        A::Details(m) => LogArg::Details(*m),
        A::Spread(ms) => {
            let mut mons = [MonId::NONE; 4];
            mons[..ms.len()].copy_from_slice(ms);
            LogArg::Spread {
                mons,
                len: ms.len() as u8,
            }
        }
    }
}

fn lower_t(t: &T) -> LogTag<'_> {
    match t {
        T::From(e) => LogTag::From(*e),
        T::Of(m) => LogTag::Of(*m),
        T::Bare(s) => LogTag::Bare(s),
        T::Val(n, a) => LogTag::Value(n, lower_a(a)),
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Stats {
    battles: usize,
    entries: usize,
    triples: usize,
    moves: usize,
    edits: usize,
    illusion_switches: usize,
    from_effects: usize,
    from_bare: usize,
}

struct Runner {
    b: Battle<TextLog>,
    id: String,
    interned: HashMap<String, &'static str>,
    pending: Vec<Edit>,
    /// Fainted mons still `isActive` while their End events run (battle.ts:2556-2570): they keep the
    /// slot ident until a line proves otherwise.
    pending_faint: Vec<MonId>,
    stats: Stats,
}

fn id_of(kind: EffectKind, name: &str) -> EffectId {
    dex::lookup(kind, name).unwrap_or_else(|| panic!("unknown {name}"))
}

fn fail(id: &str, at: usize, msg: &str) -> ! {
    panic!("battle {id}, entry {at}: {msg}")
}

impl Runner {
    fn intern(&mut self, s: &str) -> &'static str {
        if let Some(v) = self.interned.get(s) {
            return v;
        }
        let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
        self.interned.insert(s.to_owned(), leaked);
        leaked
    }

    fn set_name(&self, m: MonId) -> &str {
        &self.b.teams.sides[m.side().0 as usize].sets[m.0 as usize % 6].name
    }

    /// `pNx: Name` -> the mon in slot x, whose ident (own or Illusion disguise) must carry Name.
    fn slot_mon(&self, id: &str, at: usize, text: &str) -> MonId {
        let side = (text.as_bytes()[1] - b'1') as usize;
        let slot = (text.as_bytes()[2] - b'a') as usize;
        let name = &text[5..];
        let m = self.b.state.sides[side].active[slot];
        if m == MonId::NONE {
            fail(id, at, &format!("no mon in slot of `{text}`"));
        }
        let p = &self.b.state.pokemon[m.0 as usize];
        let shown = if p.illusion != MonId::NONE {
            self.set_name(p.illusion)
        } else {
            self.set_name(m)
        };
        if shown != name {
            fail(
                id,
                at,
                &format!("slot mon is `{shown}`, line says `{name}` ({text})"),
            );
        }
        m
    }

    /// `isActive = false`, `illusion = null`, `delete terastallized` (battle.ts:2567-2572).
    fn finalize_faint(&mut self, m: MonId) {
        if let Some(i) = self.pending_faint.iter().position(|x| *x == m) {
            self.pending_faint.swap_remove(i);
            let p = &mut self.b.state.pokemon[m.0 as usize];
            p.flags &= !mon_flags::ACTIVE;
            p.terastallized = TypeId::NONE;
            p.illusion = MonId::NONE;
            // clearVolatile (pokemon.ts:1519): setSpecies(baseSpecies)
            p.species = p.base_species;
        }
    }

    fn finalize_all_faints(&mut self) {
        while let Some(&m) = self.pending_faint.first() {
            self.finalize_faint(m);
        }
    }

    fn name_mon(&self, id: &str, at: usize, side: usize, name: &str) -> MonId {
        let found: Vec<MonId> = (0..6)
            .map(|i| MonId((side * 6 + i) as u8))
            .filter(|m| (m.0 as usize % 6) < self.b.state.sides[side].pokemon_count as usize)
            .filter(|m| self.set_name(*m) == name)
            .collect();
        if found.len() != 1 {
            fail(
                id,
                at,
                &format!(
                    "`p{}: {name}` matches {} party members",
                    side + 1,
                    found.len()
                ),
            );
        }
        found[0]
    }

    /// Any Pokemon ident: `p1a: Name` (active slot) or `p1: Name` (bench / fainted).
    fn mon_ident(&mut self, id: &str, at: usize, text: &str) -> Option<MonId> {
        let b = text.as_bytes();
        let ok_side = b.len() > 4 && b[0] == b'p' && (b[1] == b'1' || b[1] == b'2');
        if !ok_side {
            return None;
        }
        if b[2] == b':' && b[3] == b' ' {
            let m = self.name_mon(id, at, (b[1] - b'1') as usize, &text[4..]);
            // A bench-form ident proves the faint cascade is over for this mon.
            self.finalize_faint(m);
            Some(m)
        } else if (b[2] == b'a' || b[2] == b'b') && b[3] == b':' && b[4] == b' ' {
            Some(self.slot_mon(id, at, text))
        } else {
            None
        }
    }

    fn effect(&mut self, text: &str) -> Option<EffectRef> {
        for (prefix, kind) in [
            ("move: ", EffectKind::Move),
            ("ability: ", EffectKind::Ability),
            ("item: ", EffectKind::Item),
            ("pokemon: ", EffectKind::Species),
        ] {
            if let Some(name) = text.strip_prefix(prefix) {
                let id = dex::lookup(kind, name)
                    .unwrap_or_else(|| panic!("unresolvable effect `{text}`"));
                let r = EffectRef::Dex(id);
                // The rendered fullname must be the text itself (checks prefix + name casing).
                let mut out = String::new();
                crate::log::format::write_effect(&mut out, view(&self.b), r, true);
                assert_eq!(out, text, "effect fullname");
                self.stats.from_effects += 1;
                return Some(r);
            }
        }
        // Bare condition names (`brn`, `confusion`, `Spikes`, `lockedmove`, `Recoil` ...).
        let id = dex::lookup(EffectKind::Condition, text)?;
        let r = EffectRef::Dex(id);
        let mut out = String::new();
        crate::log::format::write_effect(&mut out, view(&self.b), r, true);
        if out == text {
            self.stats.from_bare += 1;
            Some(r)
        } else {
            None
        }
    }

    fn arg(&mut self, id: &str, at: usize, kind: &str, idx: usize, s: &str) -> A {
        if s.is_empty() {
            return A::Empty;
        }
        if matches!(kind, "-sidestart" | "-sideend") && idx == 0 {
            let side = (s.as_bytes()[1] - b'1') as usize;
            assert_eq!(
                s,
                format!("p{}: {}", side + 1, self.b.names[side]),
                "side ident"
            );
            return A::Side(side as u8);
        }
        if let Some(m) = self.mon_ident(id, at, s) {
            return A::Mon(m);
        }
        if s.bytes().all(|c| c.is_ascii_digit()) && s.len() < 6 {
            return A::Num(s.parse().unwrap());
        }
        A::Text(s.to_owned())
    }

    fn tag(&mut self, id: &str, at: usize, s: &str) -> T {
        if let Some(rest) = s.strip_prefix("[of] ") {
            let m = self
                .mon_ident(id, at, rest)
                .unwrap_or_else(|| fail(id, at, &format!("unresolvable [of] {rest}")));
            return T::Of(m);
        }
        if let Some(rest) = s.strip_prefix("[from] ") {
            if let Some(e) = self.effect(rest) {
                return T::From(e);
            }
            return T::Val("from", A::Text(rest.to_owned()));
        }
        if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            return T::Bare(self.intern(inner));
        }
        let (name, value) = s[1..]
            .split_once("] ")
            .unwrap_or_else(|| fail(id, at, &format!("odd tag `{s}`")));
        T::Val(self.intern(name), A::Text(value.to_owned()))
    }

    fn emit(&mut self, cmd: &str, args: &[A], tags: &[T], split: Option<u8>, move_line: bool) {
        let cmd = self.intern(cmd);
        let largs: Vec<LogArg<'_>> = args.iter().map(lower_a).collect();
        let ltags: Vec<LogTag<'_>> = tags.iter().map(lower_t).collect();
        let entry = match split {
            Some(side) => LogEntry::split(cmd, &largs, &ltags, SideId(side), false),
            None => LogEntry::new(cmd, &largs, &ltags),
        };
        if move_line {
            self.b.add_move(entry);
        } else {
            self.b.add(entry);
        }
    }

    fn flush_edits(&mut self) {
        for e in std::mem::take(&mut self.pending) {
            self.stats.edits += 1;
            match e {
                Edit::Still => self.b.attr_last_move(MoveLineEdit::Still),
                Edit::Miss => self.b.attr_last_move(MoveLineEdit::Miss),
                Edit::NoTarget => self.b.attr_last_move(MoveLineEdit::NoTarget),
                Edit::Spread(ms) => {
                    let a = A::Spread(ms);
                    self.b
                        .attr_last_move(MoveLineEdit::Tag(LogTag::Value("spread", lower_a(&a))));
                }
            }
        }
    }

    fn parse_hp(text: &str) -> (u16, u16, Status) {
        if text == "0 fnt" {
            return (0, 0, Status::Fainted);
        }
        let (frac, status) = match text.split_once(' ') {
            Some((f, s)) => (f, status_of(s)),
            None => (text, Status::None),
        };
        let (hp, max) = frac.split_once('/').unwrap();
        (hp.parse().unwrap(), max.parse().unwrap(), status)
    }

    /// One `|split|pN` triple: `switch` / `drag` (FullDetails) or `-damage` / `-heal` (Health).
    fn triple(&mut self, log: &[&str], at: usize) {
        let (side_marker, secret, public) = (log[at], log[at + 1], log[at + 2]);
        let id = self.id.clone();
        let side = (side_marker.as_bytes()[8] - b'1') as usize;
        let f: Vec<&str> = secret.split('|').collect();
        let kind = f[1];
        if matches!(kind, "switch" | "drag") {
            // Slot letters are read when attrLastMove runs, i.e. before the next switch-in moves anyone.
            self.flush_edits();
            self.finalize_all_faints();
        }
        let (hp, max, status) = Self::parse_hp(if matches!(kind, "switch" | "drag") {
            f[4].split('|').next().unwrap()
        } else {
            f[3]
        });
        // Details live in f[3], HP in f[4] for switch/drag (`|switch|IDENT|DETAILS|HP`).
        let m;
        match kind {
            "switch" | "drag" => {
                let b = f[2].as_bytes();
                assert_eq!(b[1] - b'1', side as u8);
                let slot = (b[2] - b'a') as usize;
                let name = &f[2][5..];
                let party: Vec<MonId> = (0..self.b.state.sides[side].pokemon_count as usize)
                    .map(|i| MonId((side * 6 + i) as u8))
                    .collect();
                let shown = party
                    .iter()
                    .copied()
                    .find(|m| self.set_name(*m) == name)
                    .unwrap_or_else(|| fail(&id, at, &format!("no party member named {name}")));
                // Illusion: the shown mon's max HP differs from the line's (which is the real mon's).
                let mut real = shown;
                let illusion_mon = party.iter().copied().find(|m| {
                    self.b.teams.sides[side].sets[m.0 as usize % 6].ability == dex::ABILITY_ILLUSION
                });
                if let Some(z) = illusion_mon {
                    let zmax = self.b.state.pokemon[z.0 as usize].max_hp;
                    let smax = self.b.state.pokemon[shown.0 as usize].max_hp;
                    if z != shown && max == zmax {
                        if max != smax {
                            real = z;
                        } else {
                            // Equal max HP for the disguise and Zoroark: only a later `|replace|` for this
                            // slot (before the slot's next switch-in) reveals the Illusion.
                            let marker =
                                format!("|replace|p{}{}: ", side + 1, (b'a' + slot as u8) as char);
                            let revealed = log[at + 3..]
                                .iter()
                                .take_while(|l| {
                                    !l.starts_with(&format!(
                                        "|switch|p{}{}: ",
                                        side + 1,
                                        (b'a' + slot as u8) as char
                                    ))
                                })
                                .any(|l| l.starts_with(&marker));
                            if revealed {
                                real = z;
                            }
                        }
                    }
                }
                let slot_old = self.b.state.sides[side].active[slot];
                if slot_old != MonId::NONE {
                    let p = &mut self.b.state.pokemon[slot_old.0 as usize];
                    p.flags &= !mon_flags::ACTIVE;
                    p.illusion = MonId::NONE;
                    p.species = p.base_species;
                }
                // The incoming mon may still be listed as active in the other slot (a swap).
                for other in 0..2 {
                    if self.b.state.sides[side].active[other] == real {
                        self.b.state.sides[side].active[other] = MonId::NONE;
                    }
                }
                activate(&mut self.b, real, slot as u8);
                self.b.state.pokemon[real.0 as usize].illusion =
                    if real != shown { shown } else { MonId::NONE };
                if real != shown {
                    self.stats.illusion_switches += 1;
                }
                m = real;
            }
            _ => {
                m = self
                    .mon_ident(&id, at, f[2])
                    .unwrap_or_else(|| fail(&id, at, "unresolvable HP line ident"));
            }
        }
        {
            let p = &mut self.b.state.pokemon[m.0 as usize];
            p.hp = hp;
            if hp != 0 {
                p.max_hp = max;
            }
            p.status = status;
        }
        let hp_field = if matches!(kind, "switch" | "drag") {
            5
        } else {
            4
        };
        let (a1, tag_start) = if matches!(kind, "switch" | "drag") {
            (A::Full(m), hp_field)
        } else {
            (A::Health(m), hp_field)
        };
        let tags: Vec<T> = f[tag_start.min(f.len())..]
            .iter()
            .map(|t| self.tag(&id, at, t))
            .collect();
        let before = self.b.log.entries.len();
        self.emit(kind, &[A::Mon(m), a1], &tags, Some(side as u8), false);
        let got = &self.b.log.entries[before..];
        let want = [side_marker, secret, public];
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            if g != w {
                fail(
                    &id,
                    at + i,
                    &format!("triple mismatch\n  want {w:?}\n   got {g:?}"),
                );
            }
        }
        self.stats.triples += 1;
    }

    fn line(&mut self, at: usize, line: &str) {
        let id = self.id.clone();
        let f: Vec<&str> = line.split('|').collect();
        assert_eq!(f[0], "", "entry must start with `|`: {line:?}");
        let kind = f[1];
        let body = &f[2..];
        // Lines that cannot belong to a faint cascade: the fainted mons are benched by now.
        if matches!(
            kind,
            "move" | "-anim" | "turn" | "upkeep" | "" | "t:" | "win" | "tie" | "cant"
        ) {
            self.finalize_all_faints();
        }
        match kind {
            "move" | "-anim" => return self.move_line(at, kind, body),
            _ => {}
        }
        // State that must be in place before the line is formatted.
        match kind {
            "replace" => {
                // The ident is the real mon's: look the slot up without the disguise-name check.
                let side = (body[0].as_bytes()[1] - b'1') as usize;
                let slot = (body[0].as_bytes()[2] - b'a') as usize;
                let m = self.b.state.sides[side].active[slot];
                self.b.state.pokemon[m.0 as usize].illusion = MonId::NONE;
            }
            "detailschange" => {
                let m = self.mon_ident(&id, at, body[0]).unwrap();
                let species = body[1].split(", ").next().unwrap();
                let sp = dex::lookup(EffectKind::Species, species)
                    .unwrap_or_else(|| fail(&id, at, &format!("unknown species {species}")));
                let p = &mut self.b.state.pokemon[m.0 as usize];
                p.species = sp;
                p.base_species = sp;
            }
            _ => {}
        }
        let mut args = Vec::new();
        let mut tags = Vec::new();
        let mut i = 0;
        while i < body.len() {
            let s = body[i];
            if s.starts_with('[')
                && !matches!(kind, "rule" | "-hint" | "-message" | "tier" | "win")
                && s.contains(']')
            {
                tags.push(self.tag(&id, at, s));
            } else {
                let a = match (kind, i) {
                    ("player", 0) | ("teamsize", 0) => A::SideId((s.as_bytes()[1] - b'1') as u8),
                    ("player", 1) => A::Player(if s == self.b.names[0] { 0 } else { 1 }),
                    ("detailschange" | "replace", 1) => {
                        let m = self.mon_ident(&id, at, body[0]).unwrap();
                        A::Details(m)
                    }
                    ("win", 0) => A::Player(if s == self.b.names[0] { 0 } else { 1 }),
                    ("rule" | "tier" | "-hint" | "-message", _) => A::Text(s.to_owned()),
                    _ => self.arg(&id, at, kind, i, s),
                };
                args.push(a);
            }
            i += 1;
        }
        let before = self.b.log.entries.len();
        self.emit(kind, &args, &tags, None, false);
        assert_eq!(self.b.log.entries.len(), before + 1);
        // State that changes after the line.
        match kind {
            "faint" => {
                let m = self.mon_ident(&id, at, body[0]).unwrap();
                self.pending_faint.push(m);
            }
            "-terastallize" => {
                let m = self.mon_ident(&id, at, body[0]).unwrap();
                let p = &mut self.b.state.pokemon[m.0 as usize];
                p.terastallized = dex::type_id(body[1]).unwrap();
                // Tera Morpeko-Hangry silently makes the current forme the base one (battle-actions.ts:1946-1951).
                if dex::species(p.species).base_species_name == "Morpeko" {
                    p.base_species = p.species;
                }
            }
            "-transform" => {
                // setSpecies(target.species): the transformed species is what getUpdatedDetails prints.
                let m = self.mon_ident(&id, at, body[0]).unwrap();
                let t = self.mon_ident(&id, at, body[1]).unwrap();
                self.b.state.pokemon[m.0 as usize].species =
                    self.b.state.pokemon[t.0 as usize].species;
            }
            "-formechange" => {
                let m = self.mon_ident(&id, at, body[0]).unwrap();
                if self.b.state.pokemon[m.0 as usize].illusion == MonId::NONE {
                    if let Some(sp) = dex::lookup(EffectKind::Species, body[1]) {
                        self.b.state.pokemon[m.0 as usize].species = sp;
                    }
                }
            }
            _ => {}
        }
    }

    fn move_line(&mut self, at: usize, kind: &str, body: &[&str]) {
        let id = self.id.clone();
        // Edits queued for the previous move line land before a new move line is added
        // (attrLastMove always runs before the next addMove in the real flow, except nesting,
        // where the log itself already shows the inner line carrying them).
        self.flush_edits();
        self.stats.moves += 1;
        let user = self.mon_ident(&id, at, body[0]).unwrap();
        let mut args = vec![A::Mon(user), A::Text(body[1].to_owned())];
        let mut rest = &body[2..];
        if let Some((target, tail)) = rest.split_first() {
            args.push(if target.is_empty() {
                // Blanked by `[still]`: the original target is not in the log; any mon will do.
                A::Mon(user)
            } else {
                A::Mon(self.mon_ident(&id, at, target).unwrap())
            });
            rest = tail;
        }
        let mut tags = Vec::new();
        let mut edits = Vec::new();
        for s in rest {
            match *s {
                "[still]" => edits.push(Edit::Still),
                "[miss]" => edits.push(Edit::Miss),
                "[notarget]" => edits.push(Edit::NoTarget),
                _ if s.starts_with("[spread]") => {
                    let list = s.strip_prefix("[spread] ").unwrap_or("");
                    let mons = list
                        .split(',')
                        .filter(|x| !x.is_empty())
                        .map(|slot| {
                            let side = (slot.as_bytes()[1] - b'1') as usize;
                            let idx = (slot.as_bytes()[2] - b'a') as usize;
                            self.b.state.sides[side].active[idx]
                        })
                        .collect();
                    edits.push(Edit::Spread(mons));
                }
                _ if edits.is_empty() => tags.push(self.tag(&id, at, s)),
                _ => fail(&id, at, &format!("tag `{s}` after an attr edit")),
            }
        }
        let cmd = if kind == "move" { "move" } else { "-anim" };
        self.emit(cmd, &args, &tags, None, true);
        self.pending = edits;
    }
}

fn replay_battle(header: &[&str], log: &[&str], stats: &mut Stats) {
    let [id, p1, p2, t1, t2] = header else {
        panic!("bad header {header:?}")
    };
    let b = Battle::from_players([1, 2, 3, 4], (*p1, *t1), (*p2, *t2), TextLog::default())
        .unwrap_or_else(|e| panic!("{id}: team error {e}"));
    let mut r = Runner {
        b,
        id: (*id).to_owned(),
        interned: HashMap::new(),
        pending: Vec::new(),
        pending_faint: Vec::new(),
        stats: Stats::default(),
    };
    // Zacian / Zamazenta holding their sword / shield turn Crowned in BattleStart, before the first
    // switch line and without any log line (data/conditions.ts:880-939); seed that silent change.
    for side in 0..2 {
        for i in 0..r.b.state.sides[side].pokemon_count as usize {
            let set = &r.b.teams.sides[side].sets[i];
            let crowned = match (dex::species(set.species).name, set.item) {
                ("Zacian", item) if item == id_of(EffectKind::Item, "Rusted Sword") => {
                    "Zacian-Crowned"
                }
                ("Zamazenta", item) if item == id_of(EffectKind::Item, "Rusted Shield") => {
                    "Zamazenta-Crowned"
                }
                _ => continue,
            };
            let crowned = id_of(EffectKind::Species, crowned);
            let p = &mut r.b.state.pokemon[side * 6 + i];
            p.species = crowned;
            p.base_species = crowned;
        }
    }
    // Constructor lines: realized by emit_opening_log, not replayed from the log.
    r.b.emit_opening_log();
    assert_eq!(r.b.log.entries, log[..4], "{id}: opening lines");
    let mut at = 4;
    while at < log.len() {
        let line = log[at];
        if line.starts_with("|split|") {
            r.triple(log, at);
            at += 3;
        } else {
            r.line(at, line);
            at += 1;
        }
    }
    r.flush_edits();
    // Whole-log comparison with a readable first difference.
    let got = &r.b.log.entries;
    for (i, (g, w)) in got.iter().zip(log).enumerate() {
        assert_eq!(g, w, "{id}: entry {i} differs");
    }
    assert_eq!(got.len(), log.len(), "{id}: entry count");
    // Logical line count equals the number of raw entries (no sendUpdates in between).
    assert_eq!(
        r.b.scratch.unsent_lines as usize,
        log.len(),
        "{id}: unsent count"
    );
    // Draining in uneven chunks returns the same log without losing or repeating entries.
    let mut sink = TextLog::default();
    let mut drained = Vec::new();
    let mut next_drain = 1;
    for (i, e) in got.iter().enumerate() {
        sink.entries.push(e.clone());
        if i + 1 == next_drain {
            sink.drain_into(&mut drained);
            next_drain += i % 11 + 1;
        }
    }
    sink.drain_into(&mut drained);
    sink.drain_into(&mut drained);
    assert_eq!(&drained, got, "{id}: chunked drain");
    stats.battles += 1;
    stats.entries += log.len();
    stats.triples += r.stats.triples;
    stats.moves += r.stats.moves;
    stats.edits += r.stats.edits;
    stats.illusion_switches += r.stats.illusion_switches;
    stats.from_effects += r.stats.from_effects;
    stats.from_bare += r.stats.from_bare;
}

fn replay_vectors(text: &str) -> Stats {
    let mut stats = Stats::default();
    let mut header: Vec<&str> = Vec::new();
    let mut log: Vec<&str> = Vec::new();
    for line in text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)) {
        if line.starts_with("# ") {
            continue;
        }
        if let Some(h) = line.strip_prefix("#B\t") {
            if !header.is_empty() {
                replay_battle(&header, &log, &mut stats);
            }
            header = h.split('\t').collect();
            log.clear();
        } else if !header.is_empty() {
            log.push(line);
        }
    }
    if !header.is_empty() {
        // The final newline leaves one empty trailing element.
        if log.last() == Some(&"") {
            log.pop();
        }
        replay_battle(&header, &log, &mut stats);
    }
    stats
}

#[test]
fn real_logs_replay_with_crlf() {
    let lf = VECTORS.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(replay_vectors(&lf), replay_vectors(&crlf));
}

#[test]
fn real_logs_replay_byte_for_byte() {
    let stats = replay_vectors(VECTORS);
    eprintln!("{stats:?}");
    assert!(stats.battles >= 50, "{stats:?}");
    assert!(stats.entries > 20_000, "{stats:?}");
    assert!(stats.triples > 3_000, "{stats:?}");
    assert!(
        stats.illusion_switches > 0,
        "no Illusion switch-in in the vectors"
    );
    assert!(stats.edits > 500, "{stats:?}");
}

/// `TEXTLOG_REPLAY=/path/to/extracted.tsv cargo test -p engine --lib replay_corpus_from_env -- --ignored`
#[test]
#[ignore = "needs TEXTLOG_REPLAY=<tsv from tools/probes/textlog/extract-replay.mjs>"]
fn replay_corpus_from_env() {
    let path = std::env::var("TEXTLOG_REPLAY").expect("TEXTLOG_REPLAY");
    let text = std::fs::read_to_string(path).unwrap();
    let stats = replay_vectors(&text);
    eprintln!("{stats:?}");
}
