// Shared helpers for the differential-testing oracle: loading the pinned Showdown build,
// deterministic seed derivation, and the fixture normalization rules.
//
// Only Node built-ins and Showdown's dist/ are used. The Showdown checkout is never modified.
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const FORMAT_ID = 'gen9randomdoublesbattle';
export const FIXTURE_VERSION = 1;
export const PLAYER_NAMES = ['Alice', 'Bob'];
export const SIDE_IDS = ['p1', 'p2'];

const here = path.dirname(fileURLToPath(import.meta.url));

/** Commit of the pinned Showdown checkout, from tools/oracle/ORACLE_COMMIT. */
export function readOracleCommit() {
	return fs.readFileSync(path.join(here, '..', 'ORACLE_COMMIT'), 'utf8').trim();
}

export function expandHome(p) {
	return p.startsWith('~') ? path.join(os.homedir(), p.slice(1)) : p;
}

/**
 * Loads Showdown's compiled simulator from `<psPath>/dist`. Every call of `loadSim` from a
 * different worker thread gets an independent module instance (worker threads do not share
 * the module cache), so no Dex/data state is shared between workers.
 */
export function loadSim(psPath, { checkCommit = true } = {}) {
	const root = path.resolve(expandHome(psPath));
	if (checkCommit) {
		const want = readOracleCommit();
		let have = '';
		try {
			have = execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
		} catch {
			throw new Error(`cannot run git rev-parse in ${root}`);
		}
		if (have !== want) throw new Error(`Showdown at ${root} is ${have}, but tools/oracle/ORACLE_COMMIT pins ${want}`);
	}
	const require = createRequire(path.join(root, 'package.json'));
	const sim = require('./dist/sim');
	const prng = require('./dist/sim/prng');
	return {
		root, Battle: sim.Battle, BattleStream: sim.BattleStream, Teams: sim.Teams, PRNG: sim.PRNG,
		Gen5RNG: prng.Gen5RNG, SodiumRNG: prng.SodiumRNG,
	};
}

// ---------------------------------------------------------------------------------------
// Seeds
// ---------------------------------------------------------------------------------------

/** 4 x u16 words, big-endian, from 8 bytes of `buf` starting at `off`. */
function words(buf, off) {
	return [0, 1, 2, 3].map(k => buf.readUInt16BE(off + 2 * k));
}

/**
 * All randomness of battle number `index` in run `runSeed` is derived here, from nothing else:
 *   h = SHA-256(`numbrion-oracle-v1/<runSeed>/<index>`)           (32 bytes)
 *   battleSeed  = u16 big-endian words of h[0..8]    Gen5 seed of Battle.prng
 *   teamSeeds   = [h[8..16], h[16..24]]              Gen5 seeds of the two random-team generators
 *   choiceSeed  = h[24..32]                          Gen5 seed of the choice-picking PRNG
 * Each seed is `[a, b, c, d]` with state = a<<48 | b<<32 | c<<16 | d.
 */
export function deriveSeeds(runSeed, index) {
	const h = createHash('sha256').update(`numbrion-oracle-v1/${runSeed}/${index}`).digest();
	return {
		battleSeed: words(h, 0),
		teamSeeds: [words(h, 8), words(h, 16)],
		choiceSeed: words(h, 24),
	};
}

/** Seed for the j-th team of `gen-teams.mjs`. */
export function deriveTeamSeed(runSeed, j) {
	const h = createHash('sha256').update(`numbrion-oracle-v1/teams/${runSeed}/${j}`).digest();
	return words(h, 0);
}

/** Deterministic selection of the fixtures to verify: true for a fraction `f` of indices. */
export function selectForVerify(runSeed, index, f) {
	if (f <= 0) return false;
	if (f >= 1) return true;
	const h = createHash('sha256').update(`numbrion-oracle-v1/verify/${runSeed}/${index}`).digest();
	return h.readUInt32BE(0) / 2 ** 32 < f;
}

/** Showdown PRNGSeed string ("a,b,c,d", which selects the Gen5 LCG) from 4 x u16. */
export function seedString(seed) {
	if (seed.length !== 4 || !seed.every(n => Number.isInteger(n) && n >= 0 && n <= 0xFFFF)) {
		throw new Error(`bad Gen5 seed ${JSON.stringify(seed)}`);
	}
	return seed.join(',');
}

/** Parses Showdown's `prng.getSeed()` ("a,b,c,d" for the Gen5 RNG) into 4 x u16. */
export function parseSeed(str) {
	if (!/^\d+,\d+,\d+,\d+$/.test(str)) throw new Error(`not a Gen5 seed: ${str}`);
	return str.split(',').map(Number);
}

// ---------------------------------------------------------------------------------------
// Log normalization
// ---------------------------------------------------------------------------------------

/**
 * `|t:|<unix seconds>` lines are wall clock. They are normalized to exactly `|t:|`.
 * (Showdown emits them in the Battle constructor and at the start of every turnLoop call.)
 */
export function normalizeLogLine(line) {
	return /^\|t:\|\d+$/.test(line) ? '|t:|' : line;
}

export const normalizeLog = lines => lines.map(normalizeLogLine);
