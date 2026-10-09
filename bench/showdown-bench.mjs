// Throughput benchmark for Pokemon Showdown's simulator, run in-process.
// Usage: node bench/showdown-bench.mjs <path-to-pokemon-showdown> [--seconds N] [--threads N] [--format id]
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import { createRequire } from 'node:module';
import path from 'node:path';
import os from 'node:os';

function parseArgs(argv) {
	const args = { psPath: argv[0], seconds: 20, threads: 1, format: 'gen9randomdoublesbattle', mode: 'sync' };
	for (let i = 1; i < argv.length; i += 2) args[argv[i].replace(/^--/, '')] = argv[i + 1];
	args.seconds = Number(args.seconds);
	args.threads = Number(args.threads);
	return args;
}

function loadSim(psPath) {
	const require = createRequire(path.resolve(psPath, 'package.json'));
	return {
		sim: require('./dist/sim'),
		RandomPlayerAI: require('./dist/sim/tools/random-player-ai').RandomPlayerAI,
	};
}

// Runs random-vs-random battles by calling Battle directly, with no streams or text protocol parsing.
function runSync({ psPath, seconds, format, workerId }) {
	const { sim, RandomPlayerAI } = loadSim(psPath);
	const { Battle, Teams, PRNG } = sim;
	let pending = null;
	const makeAI = seed => {
		const ai = new RandomPlayerAI({ write() {} }, { seed });
		ai.choose = choice => { pending = choice; };
		return ai;
	};
	const stats = { battles: 0, turns: 0, decisions: 0, teamGenMs: 0, battleMs: 0, errors: 0 };
	const seedPrng = new PRNG([workerId + 1, 2, 3, 4]);
	const deadline = performance.now() + seconds * 1000;
	while (performance.now() < deadline) {
		const seed = seedPrng.getSeed();
		const t0 = performance.now();
		const gen = Teams.getGenerator(format, seed);
		const teams = [Teams.pack(gen.getTeam()), Teams.pack(gen.getTeam())];
		const t1 = performance.now();
		const battle = new Battle({ formatid: format, seed, send() {} });
		battle.setPlayer('p1', { name: 'a', team: teams[0] });
		battle.setPlayer('p2', { name: 'b', team: teams[1] });
		const ais = [makeAI([seedPrng.random(65536), 1, 2, 3]), makeAI([seedPrng.random(65536), 4, 5, 6])];
		let guard = 0;
		try {
			while (!battle.ended && guard++ < 2000) {
				for (let i = 0; i < 2; i++) {
					const side = battle.sides[i];
					const req = side.activeRequest;
					if (!req || req.wait || side.isChoiceDone()) continue;
					pending = null;
					ais[i].receiveRequest(req);
					if (pending === null) continue;
					stats.decisions++;
					if (!battle.choose(side.id, pending)) side.autoChoose(), battle.allChoicesDone() && battle.commitChoices();
				}
			}
		} catch (e) {
			stats.errors++;
		}
		const t2 = performance.now();
		stats.teamGenMs += t1 - t0;
		stats.battleMs += t2 - t1;
		stats.turns += battle.turn;
		stats.battles++;
		battle.destroy();
	}
	return stats;
}

// Same battles through BattleStream and the text protocol, the way poke-env-like harnesses drive Showdown.
async function runStream({ psPath, seconds, format }) {
	const { sim, RandomPlayerAI } = loadSim(psPath);
	const { BattleStream, getPlayerStreams } = sim;
	const stats = { battles: 0, turns: 0, decisions: 0, teamGenMs: 0, battleMs: 0, errors: 0 };
	const deadline = performance.now() + seconds * 1000;
	while (performance.now() < deadline) {
		const t0 = performance.now();
		const streams = getPlayerStreams(new BattleStream());
		const p1 = new RandomPlayerAI(streams.p1);
		const p2 = new RandomPlayerAI(streams.p2);
		void p1.start();
		void p2.start();
		let turns = 0;
		const done = (async () => {
			for await (const chunk of streams.omniscient) {
				const m = chunk.match(/\|turn\|(\d+)/g);
				if (m) turns = Number(m[m.length - 1].slice(6));
			}
		})();
		void streams.omniscient.write(`>start {"formatid":"${format}"}\n>player p1 {"name":"a"}\n>player p2 {"name":"b"}`);
		await done;
		stats.battleMs += performance.now() - t0;
		stats.turns += turns;
		stats.battles++;
	}
	return stats;
}

if (!isMainThread) {
	const fn = workerData.mode === 'stream' ? runStream : runSync;
	Promise.resolve(fn(workerData)).then(s => parentPort.postMessage(s));
} else {
	const args = parseArgs(process.argv.slice(2));
	const started = performance.now();
	const results = await Promise.all(Array.from({ length: args.threads }, (_, workerId) => new Promise((resolve, reject) => {
		const w = new Worker(new URL(import.meta.url), { workerData: { ...args, workerId } });
		w.once('message', resolve);
		w.once('error', reject);
	})));
	const wall = (performance.now() - started) / 1000;
	const total = results.reduce((a, s) => {
		for (const k in s) a[k] = (a[k] || 0) + s[k];
		return a;
	}, {});
	console.log(JSON.stringify({
		mode: args.mode,
		format: args.format,
		threads: args.threads,
		wallSeconds: +wall.toFixed(1),
		battles: total.battles,
		battlesPerSec: +(total.battles / args.seconds).toFixed(1),
		turnsPerBattle: +(total.turns / total.battles).toFixed(1),
		decisionsPerBattle: +(total.decisions / total.battles).toFixed(1),
		msPerBattleSim: +(total.battleMs / total.battles).toFixed(2),
		msPerTeamPair: +(total.teamGenMs / total.battles).toFixed(2),
		errors: total.errors,
		cpus: os.cpus().length,
	}));
}
