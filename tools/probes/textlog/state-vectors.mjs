// Ground truth for the TextLog formatters that depend on Pokemon state: ident (`Pokemon.toString`),
// details (`getUpdatedDetails`, `(illusion || this).details + tera`), `getHealth` and `getFullDetails`.
// Builds a real pinned-Showdown Battle, forces the state each row describes, and records what
// Showdown prints. crates/engine/src/log/text/tests/state.rs applies the same state to the Rust
// engine and compares every column.
//
//   node tools/probes/textlog/state-vectors.mjs [<showdown checkout>] > \
//        crates/engine/src/log/text/tests/state-vectors.tsv
//
// Row: spec <TAB> ident <TAB> detailsFull <TAB> detailsPlain <TAB> hpSecret <TAB> hpShared <TAB> fullSecret <TAB> fullShared
//   spec = space separated `key=value`: m=<side>:<index> (0-based p1:0), act=a|b, hp=N, st=<status>,
//          ill=<side>:<index>, tera=<Type>, sp=<Species name> (permanent forme: species+baseSpecies+details).
import { loadSim } from '../../oracle/lib/common.mjs';

const sim = loadSim(process.argv[2] || '/home/aminaliu/src/pokemon-showdown');

const TEAMS = [
	[
		'Wugtrio||ChoiceBand|Gooey|throatchop,aquajet,stompingtantrum,liquidation||85,85,85,85,85,85|F|||92|,,,,,Dark',
		'Dondozo||AssaultVest|Unaware|heavyslam,wavecrash,bodypress,avalanche||85,85,85,85,85,85|F|||85|,,,,,Steel',
		'Gholdengo||ChoiceScarf|GoodasGold|shadowball,makeitrain,trick,focusblast||85,,85,85,85,85|N|,0,,,,||78|,,,,,Stellar',
		'Deoxys|DeoxysDefense|SitrusBerry|Pressure|knockoff,stealthrock,nightshade,thunderwave||85,85,85,85,85,85|N|||88|,,,,,Steel',
		'Zoroark||Leftovers|Illusion|nastyplot,flamethrower,knockoff,protect||85,85,85,85,85,85|M||S|84|,,,,,Dark',
		'Porygon2||Eviolite|Download|triattack,recover,thunderwave,protect||85,85,85,85,85,85|N||S|99|,,,,,Normal',
	],
	[
		'Klefki||LightClay|Prankster|dazzlinggleam,thunderwave,lightscreen,reflect||85,,85,85,85,85|F|,0,,,,||78|,,,,,Water',
		'Reshiram||LifeOrb|Turboblaze|protect,tailwind,dracometeor,heatwave||85,,85,85,85,85|N|,0,,,,||72|,,,,,Fire',
		'Basculegion|BasculegionF|ChoiceScarf|Adaptability|wavecrash,closecombat,protect,uturn||85,85,85,85,85,85|F|||88|,,,,,Water',
		'Arceus|ArceusBug|InsectPlate|Multitype|extremespeed,xscissor,swordsdance,stompingtantrum||85,85,85,85,85,85|N|||74|,,,,,Normal',
		'Ogerpon|OgerponWellspring|WellspringMask|WaterAbsorb|ivycudgel,knockoff,swordsdance,hornleech||85,85,85,85,85,85|F||S|80|,,,,,Water',
		'Greninja|GreninjaBond|ChoiceSpecs|BattleBond|hydropump,icebeam,darkpulse,uturn||85,85,85,85,85,85|M|||100|,,,,,Water',
	],
];
const NAMES = ['Alice', 'Bob'];

const battle = new sim.Battle({ formatid: 'gen9randomdoublesbattle', seed: '1,2,3,4' });
battle.setPlayer('p1', { name: NAMES[0], team: TEAMS[0].join(']') });
battle.setPlayer('p2', { name: NAMES[1], team: TEAMS[1].join(']') });
const mon = (ref) => {
	const [s, i] = ref.split(':').map(Number);
	return battle.sides[s].pokemon[i];
};

// Snapshot/restore the fields a row mutates, so rows are independent.
const KEYS = ['isActive', 'position', 'hp', 'status', 'illusion', 'terastallized', 'species', 'baseSpecies', 'details'];
const snap = (p) => Object.fromEntries(KEYS.map(k => [k, p[k]]));
const restore = (p, s) => { for (const k of KEYS) p[k] = s[k]; };

function row(spec) {
	const kv = Object.fromEntries(spec.split(' ').map(x => x.split('=')));
	const p = mon(kv.m);
	const saved = snap(p);
	// The real battle already started (mons 0 and 1 of each side are active): every row states its own activity.
	p.isActive = !!kv.act;
	if (kv.act) p.position = kv.act === 'a' ? 0 : 1;
	if (kv.hp !== undefined) p.hp = Number(kv.hp);
	if (kv.st) p.status = kv.st;
	if (kv.ill) p.illusion = mon(kv.ill);
	if (kv.tera) p.terastallized = kv.tera;
	if (kv.sp) {
		const sp = battle.dex.species.get(kv.sp);
		p.species = sp; p.baseSpecies = sp; p.details = p.getUpdatedDetails();
	}
	const health = p.getHealth();
	const full = p.getFullDetails();
	const detailsFull = (p.illusion || p).details + (p.terastallized ? `, tera:${p.terastallized}` : '');
	const out = [spec, p.toString(), detailsFull, p.getUpdatedDetails(), health.secret, health.shared, full.secret, full.shared];
	restore(p, saved);
	return out.join('\t');
}

const rows = [];
// Baseline: every mon inactive, then active in slot a and b, full HP.
for (let s = 0; s < 2; s++) for (let i = 0; i < 6; i++) {
	rows.push(`m=${s}:${i}`);
	rows.push(`m=${s}:${i} act=a`);
	rows.push(`m=${s}:${i} act=b`);
}
// HP grid on three max-HP values (all hp), covering the 99/100 clamp, tiny HP and faint.
for (const ref of ['0:0', '1:3', '0:5']) {
	const max = mon(ref).maxhp;
	for (let hp = 0; hp <= max; hp++) rows.push(`m=${ref} act=a hp=${hp}`);
}
// Status words on both views; `0 fnt` never carries one.
for (const st of ['brn', 'par', 'slp', 'frz', 'psn', 'tox']) {
	for (const ref of ['0:0', '1:1', '0:3']) {
		const max = mon(ref).maxhp;
		for (const hp of [0, 1, 2, Math.floor(max / 2), max - 1, max]) rows.push(`m=${ref} act=b hp=${hp} st=${st}`);
	}
}
// Tera suffix (including Stellar), active and inactive, with and without status.
for (const [ref, tera] of [['0:0', 'Dark'], ['0:2', 'Stellar'], ['0:3', 'Steel'], ['1:4', 'Water'], ['1:5', 'Water'], ['0:4', 'Dark']]) {
	rows.push(`m=${ref} act=a tera=${tera}`);
	rows.push(`m=${ref} tera=${tera}`);
	rows.push(`m=${ref} act=b tera=${tera} hp=7 st=brn`);
}
// Illusion: disguise ident/details/level/gender/shiny, but the real mon's HP; tera suffix of the real mon.
for (const [ref, ill] of [['0:4', '0:5'], ['0:4', '0:0'], ['0:4', '0:3'], ['0:4', '0:2'], ['0:4', '0:1']]) {
	for (const act of ['a', 'b']) {
		rows.push(`m=${ref} act=${act} ill=${ill}`);
		rows.push(`m=${ref} act=${act} ill=${ill} hp=33`);
		rows.push(`m=${ref} act=${act} ill=${ill} hp=33 st=par`);
		rows.push(`m=${ref} act=${act} ill=${ill} tera=Dark`);
		rows.push(`m=${ref} act=${act} ill=${ill} hp=0`);
	}
	rows.push(`m=${ref} ill=${ill}`); // disguise on a mon that is no longer active
}
// Permanent forme changes keep the ident name, change the details species.
for (const [ref, sp] of [['1:4', 'Ogerpon-Wellspring-Tera'], ['1:4', 'Ogerpon'], ['1:2', 'Basculegion'], ['1:5', 'Greninja-Bond'],
	['0:3', 'Deoxys'], ['0:3', 'Deoxys-Attack'], ['1:3', 'Arceus'], ['1:3', 'Arceus-Fairy'], ['0:1', 'Palafin-Hero'], ['0:1', 'Mimikyu-Busted']]) {
	rows.push(`m=${ref} act=a sp=${sp}`);
	rows.push(`m=${ref} sp=${sp}`);
	rows.push(`m=${ref} act=b sp=${sp} tera=Water hp=5`);
}

let out = '# generated by tools/probes/textlog/state-vectors.mjs from the pinned Showdown; do not edit\n';
out += `#NAMES\t${NAMES.join('\t')}\n`;
out += `#TEAM\t${TEAMS[0].join(']')}\n#TEAM\t${TEAMS[1].join(']')}\n`;
for (const r of rows) out += row(r) + '\n';
process.stdout.write(out);
process.stderr.write(`${rows.length} rows\n`);
