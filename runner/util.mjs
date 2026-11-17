// Small helpers shared by the runner modules (no dependencies beyond Node).
import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';

/** The qapilot checkout: bin/, prompts/, templates/. */
export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
/** The qapilot launcher (builds the engine on first use). */
export const QAPILOT = path.join(ROOT, 'bin', 'qapilot');
/** Where qapilot keeps its data: $QAPILOT_HOME, else ~/.qapilot (the engine uses the same folder). */
export const home = (env = process.env) => env.QAPILOT_HOME || path.join(env.HOME || '', '.qapilot');

export const log = (...a) => console.log(new Date().toISOString(), ...a);

export function readJson(file, fallback) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return fallback;
  }
}

export function writeJson(file, data, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n', mode ? { mode } : undefined);
}

/** Runs a command; resolves { err, stdout, stderr } and never rejects. */
export function exec(cmd, args, { timeout = 60_000, env, cwd } = {}) {
  return new Promise((resolve) => {
    execFile(cmd, args, { timeout, env, cwd, maxBuffer: 64 * 1024 * 1024 }, (err, stdout, stderr) => resolve({ err, stdout: String(stdout || ''), stderr: String(stderr || '') }));
  });
}
