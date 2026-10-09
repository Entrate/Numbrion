// File helpers for the trace tools: (gzipped) JSONL readers and writers.
import fs from 'node:fs';
import readline from 'node:readline';
import zlib from 'node:zlib';

function isGzip(file) {
	const fd = fs.openSync(file, 'r');
	try {
		const magic = Buffer.alloc(2);
		const n = fs.readSync(fd, magic, 0, 2, 0);
		return n === 2 && magic[0] === 0x1f && magic[1] === 0x8b;
	} finally {
		fs.closeSync(fd);
	}
}

/** Streams the non-blank lines of a plain or gzipped (detected by magic bytes) text file. */
export async function* readLines(file) {
	let input = fs.createReadStream(file);
	if (isGzip(file)) input = input.pipe(zlib.createGunzip());
	const rl = readline.createInterface({ input, crlfDelay: Infinity });
	try {
		for await (const line of rl) if (line.trim() !== '') yield line;
	} finally {
		rl.close();
		input.destroy();
	}
}

/** All non-blank lines of a (gzipped) text file, in memory. */
export function readAllLines(file) {
	let buf = fs.readFileSync(file);
	if (buf.length >= 2 && buf[0] === 0x1f && buf[1] === 0x8b) buf = zlib.gunzipSync(buf);
	return buf.toString('utf8').split('\n').filter(l => l.trim() !== '');
}

/**
 * Finds one fixture line: by 0-based position among the non-blank lines (`{ position }`, what
 * `difftest --battle` uses) or by the fixture's `index` field (`{ index }`).
 * Returns { line, position } or null.
 */
export async function findFixture(file, { position, index }) {
	let pos = 0;
	for await (const line of readLines(file)) {
		if (position !== undefined) {
			if (pos === position) return { line, position: pos };
		} else {
			const m = /"index":(\d+),/.exec(line.slice(0, 600));
			if (m && Number(m[1]) === index) return { line, position: pos };
		}
		pos++;
	}
	return null;
}

/** Writes lines (each without newline) to a file; `.gz` is gzip compressed, `-` is stdout. */
export function writeLines(file, lines) {
	const text = lines.length ? `${lines.join('\n')}\n` : '';
	if (file === '-') {
		fs.writeSync(1, text);
	} else if (file.endsWith('.gz')) {
		fs.writeFileSync(file, zlib.gzipSync(text, { level: 6 }));
	} else {
		fs.writeFileSync(file, text);
	}
}
