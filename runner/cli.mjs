#!/usr/bin/env node
// qapilot-run — an AI agent (Claude Code) tests a mobile app on a device the way a tester would and writes a report.
//   qapilot-run run --app <profile.json> --user <id> (--issue <file.md>… | --issues <dir> | --flow "what to check" | --explore ["areas first"])
//                   [--device <adb serial | iOS UDID>] [--avd <name>] [--figma <url>…] [--channel <name>] [--install] [--onboard]
//                   [--note "…"] [--runs <dir>] [--model <model>] [--fg]
//   qapilot-run resume <run dir> --answer "…"     a reply to a run that stopped with "needs-input"
//   qapilot-run combine <android run> <ios run> [--runs <dir>] [--issue-url "https://tracker/browse/{key}"]
// A run detaches (its own process group) and prints its folder; --fg keeps it in the terminal.
// Runs go to --runs, else $QAPILOT_HOME/runs (~/.qapilot/runs). Secrets come from the environment: TEST_PASSWORD,
// TEST_PIN, TEST_PIN_<USER> (other users), QAPILOT_OTP_CMD — or from $QAPILOT_HOME/.env.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { home, log, readJson } from './util.mjs';
import { DEVICE_RE, FIGMA_RE } from './result.mjs';
import { run, resume, readIssue, runId, runLabel } from './run.mjs';
import { combine } from './combine.mjs';

const USAGE = `usage:
  qapilot-run run --app <profile.json> --user <id> (--issue <file.md>… | --issues <dir> | --flow "…" | --explore ["…"]) [--device …] [--figma <url>…] [--install] [--onboard] [--note "…"] [--runs <dir>] [--fg]
  qapilot-run resume <run dir> --answer "…"
  qapilot-run combine <android run> <ios run> [--runs <dir>] [--issue-url "https://tracker/browse/{key}"]`;

export function parseArgs(argv) {
  const a = { cmd: argv[0], pos: [], issues: [], figma: [], install: false, onboard: false, fg: false, note: '' };
  for (let i = 1; i < argv.length; i++) {
    const v = argv[i];
    const next = () => {
      if (argv[i + 1] === undefined) throw new Error(`${v} needs a value`);
      return argv[++i];
    };
    if (v === '--app') a.app = next();
    else if (v === '--user') a.user = next();
    else if (v === '--issue') a.issues.push(next());
    else if (v === '--issues') a.issueDir = next();
    else if (v === '--flow') a.flow = String(next()).trim();
    // The whole app; the words after it (optional) are the areas to focus on.
    else if (v === '--explore') {
      a.explore = true;
      if (argv[i + 1] !== undefined && !argv[i + 1].startsWith('--')) a.focus = String(argv[++i]).trim();
    } else if (v === '--figma') a.figma.push(String(next()).trim());
    else if (v === '--device') a.device = String(next()).trim();
    else if (v === '--avd') a.avd = next();
    else if (v === '--channel') a.channel = next();
    else if (v === '--install') a.install = true;
    else if (v === '--onboard') a.onboard = true;
    else if (v === '--note') a.note = next();
    else if (v === '--runs') a.runs = next();
    else if (v === '--model') a.model = next();
    else if (v === '--answer') a.answer = next();
    else if (v === '--issue-url') a.issueUrl = next();
    else if (v === '--fg') a.fg = true;
    else if (!v.startsWith('--')) a.pos.push(v);
    else throw new Error(`unknown argument: ${v}\n${USAGE}`);
  }
  if (a.cmd === 'run') {
    const sources = [a.issues.length || a.issueDir ? 1 : null, a.flow, a.explore].filter(Boolean).length;
    if (!a.app || !a.user || sources !== 1) throw new Error(USAGE);
    if (a.flow && a.flow.length > 4000) throw new Error('The flow description is too long (4000 characters at most)');
    if (a.focus && a.focus.length > 2000) throw new Error('The focus of a whole-app run is too long (2000 characters at most)');
    if (a.device !== undefined && !DEVICE_RE.test(a.device)) throw new Error(`Not a device: ${a.device} (an adb serial such as emulator-5554, or an iPhone / simulator UDID)`);
    const badLink = a.figma.find((u) => !FIGMA_RE.test(u));
    if (badLink) throw new Error(`Not a Figma design link: ${badLink}`);
  } else if (a.cmd === 'resume') {
    if (a.pos.length !== 1 || !a.answer?.trim()) throw new Error(USAGE);
  } else if (a.cmd === 'combine') {
    if (a.pos.length !== 2) throw new Error(USAGE);
  } else throw new Error(USAGE);
  return a;
}

/** Issue files from --issue and --issues (*.md in the folder, or <folder>/<KEY>/issue.md). */
export function issueFiles(a) {
  const files = [...a.issues];
  if (a.issueDir) {
    for (const f of fs.readdirSync(a.issueDir).sort()) {
      const p = path.join(a.issueDir, f);
      if (f.endsWith('.md')) files.push(p);
      else if (fs.existsSync(path.join(p, 'issue.md'))) files.push(path.join(p, 'issue.md'));
    }
  }
  return files.map(readIssue);
}

/** KEY=value lines of $QAPILOT_HOME/.env, under what the environment already has. */
function withEnvFile(env) {
  const file = path.join(home(env), '.env');
  if (!fs.existsSync(file)) return env;
  const out = { ...env };
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    const m = line.match(/^([A-Z_][A-Z0-9_]*)=(.*)$/);
    if (m && out[m[1]] === undefined) out[m[1]] = m[2];
  }
  return out;
}

async function main(argv) {
  const a = parseArgs(argv);
  const env = withEnvFile(process.env);
  if (a.cmd === 'combine') return combine(path.resolve(a.runs || path.join(home(env), 'runs')), a.pos[0], a.pos[1], { issueUrl: a.issueUrl });

  const runs = path.resolve(a.runs || path.join(home(env), 'runs'));
  let dir;
  let source;
  if (a.cmd === 'resume') {
    dir = path.resolve(a.pos[0].includes(path.sep) ? a.pos[0] : path.join(runs, a.pos[0]));
    if (!fs.existsSync(path.join(dir, 'run.json'))) throw new Error(`No run in ${dir}`);
  } else {
    source = { issues: issueFiles(a), flow: a.flow, explore: a.explore, focus: a.focus };
    if (!source.flow && !source.explore && !source.issues.length) throw new Error('No issues to test');
    // The background process takes the parent's folder: recomputing the id from the clock can land on the next second.
    dir = a.fg && process.env.QAPILOT_RUN_DIR ? process.env.QAPILOT_RUN_DIR : path.join(runs, runId(runLabel(source)));
  }

  if (!a.fg) {
    fs.mkdirSync(dir, { recursive: true });
    const out = fs.openSync(path.join(dir, 'run.log'), 'a');
    // Its own process group: stopping a run kills the group (runner, agent, Maestro) and nothing else.
    spawn(process.execPath, [fileURLToPath(import.meta.url), ...argv, '--fg'], { detached: true, stdio: ['ignore', out, out], env: { ...process.env, QAPILOT_RUN_DIR: dir } }).unref();
    console.log(`${path.basename(dir)} ${a.cmd === 'resume' ? 'resumed with your answer' : 'running'} in the background\n  log:    ${path.join(dir, 'run.log')}\n  report: ${path.join(dir, 'report.md')}`);
    return;
  }

  const claude = { model: a.model, cwd: runs };
  let result;
  if (a.cmd === 'resume') result = await resume({ dir, answer: a.answer, env, claude });
  else {
    const profile = path.resolve(a.app);
    const app = readJson(profile, null);
    if (!app?.id) throw new Error(`${profile}: not an app profile (JSON with at least "id" and "package" or "ios.bundleId")`);
    const users = Object.entries(app.testData || {}).map(([id, label]) => ({ id, label }));
    result = await run({
      dir, app, appDir: path.dirname(profile), user: a.user, users, source, device: a.device || env.DEVICE, avd: a.avd || env.QAPILOT_AVD,
      channel: a.channel, install: a.install, onboard: a.onboard, note: a.note, figma: a.figma, env,
      notes: app.notesFile ? path.resolve(path.dirname(profile), app.notesFile) : undefined, claude,
    });
  }
  const count = (v) => result.items.filter((i) => i.verdict === v).length;
  log(`${path.basename(dir)}: ${result.status}${result.items.length ? ` — ${count('pass')} pass · ${count('fail')} fail · ${count('blocked')} blocked` : ''}${result.findings.length ? ` · ${result.findings.length} finding(s)` : ''}`);
  log(`report: ${path.join(dir, 'report.md')}`);
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
