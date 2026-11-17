// One test run on a device: the device up, the app installed and logged in, then an AI agent (Claude Code) tests it the
// way a careful tester would and writes report.md + result.json into the run folder. Nothing here knows an issue
// tracker: issues come in as Markdown files, and whatever is special about an app comes from its profile and hooks.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { ROOT, QAPILOT, home, log, readJson, writeJson, exec } from './util.mjs';
import { ID_RE, IOS_UDID_RE, normalizeResult, saveFlows } from './result.mjs';

const FIGMA_MCP = { type: 'http', url: 'https://mcp.figma.com/mcp' };
const SDK = process.env.ANDROID_HOME || path.join(os.homedir(), 'Library', 'Android', 'sdk');
const ADB = path.join(SDK, 'platform-tools', 'adb');
const FIGMA_PNG = path.join(ROOT, 'bin', 'figma-png');
export const MAX_ISSUES = 30;
const INSTALL_CHECK_MIN = 30;
const DEFAULT_LOGIN = 'At a PIN / lock screen: `digits $TEST_PIN`. At the login screen: type $TEST_USER and $TEST_PASSWORD; `otpmark` before the step that sends an SMS code, then `otp $TEST_USER`.';

/** The label of a run id: the one issue, ISSUES, FLOW (a flow the tester described) or APP (the whole app). */
export function runLabel({ issues = [], flow, explore, label }) {
  if (label) return label;
  if (explore) return 'APP';
  if (flow) return 'FLOW';
  return issues.length === 1 ? String(issues[0].key ?? issues[0]) : 'ISSUES';
}
/** <label>-<UTC time>, e.g. PRJ-12-2026-11-17T09-30-00. */
export const runId = (label, now = new Date()) => `${label}-${now.toISOString().replace(/[:.]/g, '-').slice(0, 19)}`;

/**
 * An issue to test, from a Markdown file: the first heading `# <ID>: <title>` (else the file name is the id), and an
 * optional `- Type: … · Status: …` line. The agent reads the whole file — steps, expected results, screenshots next to it.
 */
export function readIssue(file) {
  const md = fs.readFileSync(file, 'utf8');
  const head = md.match(/^# ([A-Za-z0-9_-]+): (.+)$/m);
  const base = path.basename(file, path.extname(file));
  const key = (head?.[1] || (base === 'issue' ? path.basename(path.dirname(file)) : base)).toUpperCase();
  if (!ID_RE.test(key)) throw new Error(`${file}: no issue id (start the file with "# PRJ-12: what is wrong")`);
  const meta = md.match(/^- Type: ([^·\n]+?) · Status: ([^·\n]+?)(?: · |$)/m) || [];
  return { key, summary: (head?.[2] || '').trim(), type: (meta[1] || '').trim(), status: (meta[2] || '').trim(), file: path.resolve(file) };
}

/** Emulator snapshot of a logged-in test user: `<app>-<user>`. */
export const snapshotName = (app, user) => `${app}-${user}`.replace(/[^\w.-]/g, '_');

async function snapshots(env) {
  const r = await exec(QAPILOT, ['snap', 'list'], { timeout: 3 * 60_000, env });
  return new Set(r.err ? [] : r.stdout.split('\n').map((l) => l.trim().split(/\s+/)[1]).filter(Boolean));
}

/**
 * The app's other test users a run can switch to in seconds (a saved snapshot of the same build), with what each one
 * has: an issue that needs data this user lacks is tested with the user that has it instead of coming out "blocked".
 */
export function otherUsers(users, app, current, saved, sameBuild) {
  return (users || [])
    .filter((u) => String(u.id) !== String(current) && (!u.app || u.app === app.id) && saved.has(snapshotName(app.id, u.id)) && sameBuild(snapshotName(app.id, u.id)))
    .map((u) => ({ id: String(u.id), snapshot: snapshotName(app.id, u.id), data: String(app.testData?.[u.id] || u.label || '') }));
}

// Checking a distribution channel for a newer build can take a minute even when nothing changed: once per 30 min.
export function installCheckFresh(app, channel, version, now = Date.now(), file = path.join(home(), 'install-check.json')) {
  const c = readJson(file, {})[`${app}|${channel}`];
  return !!c && !!version && c.version === version && now - c.at < INSTALL_CHECK_MIN * 60_000;
}
function installCheckSave(app, channel, version) {
  const file = path.join(home(), 'install-check.json');
  writeJson(file, { ...readJson(file, {}), [`${app}|${channel}`]: { at: Date.now(), version } });
}

// Which build each snapshot holds, so a run without --onboard restores it only when the installed build is the same.
export const snapshotMatches = (snap, build, file = path.join(home(), 'snapshots.json')) => !!build && readJson(file, {})[snap]?.build === build;
function snapshotSaved(snap, build) {
  const file = path.join(home(), 'snapshots.json');
  writeJson(file, { ...readJson(file, {}), [snap]: { at: Date.now(), build } });
}

async function appBuild(device, pkg) {
  const r = await exec(ADB, ['-s', device, 'shell', 'dumpsys', 'package', pkg], { timeout: 20_000 });
  const name = r.stdout.match(/versionName=(\S+)/)?.[1];
  const code = r.stdout.match(/versionCode=(\d+)/)?.[1];
  return name ? `${name} (${code || '?'})` : '';
}
const appVersion = async (device, pkg) => (await exec(ADB, ['-s', device, 'shell', 'dumpsys', 'package', pkg], { timeout: 20_000 })).stdout.match(/versionName=(\S+)/)?.[1] || '';

/** "2.3.0 (230)" of an app on an iPhone / simulator, '' when it is not installed. */
async function iosAppVersion(udid, bundle) {
  const out = path.join(os.tmpdir(), `qapilot-ios-apps-${process.pid}.json`);
  await exec('xcrun', ['devicectl', 'device', 'info', 'apps', '--device', udid, '--bundle-id', bundle, '--json-output', out], { timeout: 60_000 });
  const a = readJson(out, {})?.result?.apps?.[0];
  fs.rmSync(out, { force: true });
  return a ? `${a.version} (${a.bundleVersion})` : '';
}

const bootDone = async (device) => (await exec(ADB, ['-s', device, 'shell', 'getprop', 'sys.boot_completed'], { timeout: 10_000 })).stdout.trim() === '1';

/** The emulator, started (in its own process group, so cancelling a run never kills it) when it is not up. */
async function ensureDevice(device, avd) {
  if (await bootDone(device)) return device;
  if (!avd) throw new Error(`${device} is not running — start it, or give the AVD to start (--avd)`);
  log(`starting emulator ${avd}`);
  spawn(path.join(SDK, 'emulator', 'emulator'), ['-avd', avd], { detached: true, stdio: 'ignore' }).unref();
  for (let i = 0; i < 90; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    if (await bootDone(device)) return device;
  }
  throw new Error(`emulator ${avd} did not boot in 3 minutes`);
}

/** A command whose output goes to the run's log; resolves with its exit code. */
function sh(cmd, args, env, { timeoutMs = 20 * 60_000, cwd } = {}) {
  return new Promise((resolve) => {
    const c = spawn(cmd, args, { cwd, env, stdio: ['ignore', 'inherit', 'inherit'] });
    const t = setTimeout(() => c.kill('SIGTERM'), timeoutMs);
    c.on('close', (code) => { clearTimeout(t); resolve(code); });
    c.on('error', () => { clearTimeout(t); resolve(127); });
  });
}

/** The environment the device tools, Maestro and the agent run with: SDK tools on PATH, the device, the test user. */
async function runEnv(o) {
  const env = { ...(o.env || process.env), MAESTRO_CLI_NO_ANALYTICS: '1', MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED: 'true' };
  env.PATH = [path.join(SDK, 'platform-tools'), path.join(SDK, 'emulator'), '/opt/homebrew/bin', '/usr/local/bin', env.PATH].join(':');
  if (!env.JAVA_HOME) {
    const r = await exec('/usr/libexec/java_home', [], { timeout: 10_000 });
    if (!r.err) env.JAVA_HOME = r.stdout.trim();
  }
  env.QAPILOT_HOME = home(env);
  env.TEST_USER = String(o.user);
  if (o.app?.sameWords?.length && !env.QAPILOT_SAME_WORDS) env.QAPILOT_SAME_WORDS = o.app.sameWords.join(',');
  return env;
}

function claudeArgs(o, dir, device, resume, sessionId) {
  const c = o.claude || {};
  const allowed = [
    'Read', 'Write', 'Edit', 'Glob', 'Grep', 'Skill', 'mcp__qapilot', 'mcp__maestro', 'mcp__figma',
    'Bash(maestro:*)', 'Bash(adb:*)', 'Bash(node:*)', 'Bash(ls:*)', 'Bash(cat:*)', 'Bash(mkdir:*)', 'Bash(cp:*)', 'Bash(mv:*)', 'Bash(sleep:*)', 'Bash(date:*)',
    ...[QAPILOT, FIGMA_PNG, ...(c.allowBash || [])].map((b) => `Bash(${b}:*)`),
  ];
  // The device's app data is the test user's onboarding: never wiped by a test.
  const denied = ['Bash(git:*)', 'Bash(adb uninstall:*)', 'Bash(adb shell pm uninstall:*)', 'Bash(adb shell pm clear:*)', `Bash(adb -s ${device} uninstall:*)`];
  const dirs = [...new Set([path.dirname(dir), home(o.env), ROOT, o.appDir, o.notes && path.dirname(o.notes), ...(c.addDirs || [])].filter(Boolean))];
  return [
    '-p',
    ...(resume ? ['--resume', resume] : sessionId ? ['--session-id', sessionId] : []),
    '--permission-mode', 'acceptEdits',
    '--output-format', 'json',
    ...(c.model ? ['--model', c.model] : []),
    '--mcp-config', path.join(dir, 'mcp.json'), '--strict-mcp-config',
    '--allowedTools', ...(c.allowedTools || allowed),
    '--disallowedTools', ...denied,
    '--add-dir', ...dirs,
  ];
}

// The agent keys its sessions by working directory: keep it the same for every turn of a run (claude.cwd).
function runClaude(o, dir, args, prompt, env) {
  return new Promise((resolve) => {
    const c = spawn('claude', args, { cwd: o.claude?.cwd || path.dirname(dir), env, stdio: ['pipe', 'pipe', 'inherit'] });
    c.stdout.pipe(fs.createWriteStream(path.join(dir, 'claude.json')));
    c.stdin.end(prompt);
    const timer = setTimeout(() => c.kill('SIGTERM'), (o.claude?.timeoutMinutes ?? 120) * 60_000);
    c.on('close', (code) => { clearTimeout(timer); resolve(code); });
    c.on('error', () => { clearTimeout(timer); resolve(127); });
  });
}

function playbook(device, notes) {
  return fs.readFileSync(path.join(ROOT, 'prompts', 'playbook.md'), 'utf8')
    .replaceAll('{QAPILOT}', QAPILOT).replaceAll('{FIGMA_PNG}', FIGMA_PNG).replaceAll('{DEVICE}', device)
    .replaceAll('{NOTES}', notes || '(none for this app)');
}

function sourceText(s) {
  if (s.text) return s.text;
  if (s.explore) return `Whole app — test every area of the app (playbook: "The whole app").${s.focus ? `\nFOCUS: ${s.focus}` : ''}`;
  if (s.flow) return `Flow described by the tester:\n${s.flow}`;
  return `Issues: ${(s.issues || []).map((i) => i.key ?? i).join(', ')}`;
}

/** The end of every turn: the agent's session kept, flows cut from the journal, the result normalised. */
function finish(dir, meta, code) {
  const resultFile = path.join(dir, 'result.json');
  const sid = readJson(path.join(dir, 'claude.json'), {})?.session_id;
  if (sid && meta.sessions.at(-1) !== sid) meta.sessions.push(sid);
  meta.finished = new Date().toISOString();
  delete meta.pid;
  writeJson(path.join(dir, 'run.json'), meta);
  if (!fs.existsSync(resultFile)) writeJson(resultFile, { status: 'failed', summary: `the agent exited ${code} without writing result.json`, items: [] });
  try {
    const n = saveFlows(dir, meta);
    if (n) log(`${n} replay flow(s) saved from the steps the run performed`);
  } catch (e) {
    log('replay flows:', e.message);
  }
  return normalizeResult(readJson(resultFile, null));
}

/**
 * A run, in the foreground (a CLI detaches it first). Options:
 *   dir            the run folder (its name is the run id)
 *   app, appDir    the app profile and the folder its relative paths (onboarding flow, install command) start from
 *   user, users    the test user, and the others a run may switch to ([{ id, label }], Android snapshots)
 *   source         { issues: [readIssue(…)], flow, explore, focus, label, text }
 *   device, avd    adb serial or iOS UDID (default $DEVICE, else emulator-5554); the AVD to boot if it is not up
 *   channel, install, onboard, note, figma
 *   env            the run's environment: TEST_PASSWORD, TEST_PIN, TEST_PIN_<ID>, QAPILOT_OTP_CMD, QAPILOT_HOME…
 *   notes, login   a Markdown file with what the agent must know about the app; how to get past its login
 *   installCmd     argv for `install` (default: the profile's `install`), run with APP_ID, CHANNEL, DEVICE set
 *   onboardCmd     argv for `onboard` (default: `maestro test` of the profile's `onboarding` flow)
 *   claude         { model, timeoutMinutes, cwd, allowBash, allowedTools, addDirs, figma }
 * Returns the normalised result.
 */
export async function run(o) {
  const dir = path.resolve(o.dir);
  fs.mkdirSync(dir, { recursive: true });
  const id = path.basename(dir);
  const resultFile = path.join(dir, 'result.json');
  const meta = readJson(path.join(dir, 'run.json'), { sessions: [], answers: [] });
  meta.pid = process.pid;
  let code = null;
  try {
    const app = o.app;
    if (!app?.id) throw new Error('an app profile with an id is required');
    const source = o.source || {};
    const channel = o.channel || app.defaultChannel || '';
    Object.assign(meta, {
      label: runLabel(source), source: { issues: (source.issues || []).map((i) => i.key ?? i), flow: source.flow || null, explore: !!source.explore, focus: source.focus || '', ...(source.extra || {}) },
      app: app.id, appName: app.name || app.id, package: app.package, channel, user: o.user, note: o.note || '', install: !!o.install, onboard: !!o.onboard,
      issues: (source.issues || []).slice(0, o.maxIssues || MAX_ISSUES), version: '', device: o.device || o.env?.DEVICE || 'emulator-5554', started: new Date().toISOString(),
    });
    const ios = IOS_UDID_RE.test(meta.device);
    meta.platform = ios ? 'ios' : 'android';
    if (ios) {
      if (!app.ios?.bundleId) throw new Error(`${meta.appName} has no iOS bundle id (profile → ios.bundleId)`);
      meta.package = app.ios.bundleId;
    } else if (!app.package) throw new Error(`${meta.appName} has no Android package (profile → package)`);
    meta.figma = [...new Set([...(app.figma || []).map((f) => f.url ?? f), ...(o.figma || [])])];
    const walk = !!(source.flow || source.explore); // no issues: the findings are the result
    if (!walk && !meta.issues.length) throw new Error(`No issues to test (${sourceText(source)})`);
    writeJson(path.join(dir, 'run.json'), meta);
    const env = await runEnv(o);
    env.DEVICE = meta.device;
    if (ios) env.QAPILOT_IOS_APPS = meta.package;

    if (ios) {
      // The iPhone keeps its login in the app; no snapshots. The qapilot agent must answer before the run starts.
      log(`iOS device ${meta.device}: starting the qapilot agent`);
      if ((await sh(QAPILOT, ['up'], env, { timeoutMs: 4 * 60_000 })) !== 0) throw new Error(`The qapilot agent did not start on ${meta.device} — is the iPhone connected by cable and unlocked?`);
      if (o.install) log('install on iOS: install the build yourself first (not automated)');
      meta.version = await iosAppVersion(meta.device, meta.package);
      if (!meta.version) throw new Error(`${meta.package} is not installed on ${meta.device}`);
    } else meta.device = await ensureDevice(meta.device, o.avd);
    // A fresh onboarding is an emulator snapshot once one exists (seconds instead of minutes). It is restored BEFORE
    // installing, so a newer build lands on top of the logged-in user (an update keeps the login).
    const snap = snapshotName(app.id, o.user);
    let restored = false;
    const build = ios ? '' : await appBuild(meta.device, app.package);
    if (!ios && (o.onboard || snapshotMatches(snap, build)) && (await snapshots(env)).has(snap)) {
      log(`${o.onboard ? `onboarding ${o.user}: ` : ''}restoring snapshot ${snap}`);
      restored = (await sh(QAPILOT, ['snap', 'load', snap], env, { timeoutMs: 2 * 60_000 })) === 0;
      if (!restored) log(`snapshot ${snap} could not be restored — onboarding with the flow instead`);
      // The app process in the snapshot holds a session the server forgot long ago: start it fresh.
      else await sh(QAPILOT, ['do', `stop ${app.package}`, `launch ${app.package}`], env, { timeoutMs: 60_000 });
    }
    const installCmd = o.installCmd || (app.install ? [].concat(app.install) : null);
    if (o.install && !ios) {
      if (!installCmd) throw new Error(`${meta.appName} has no install command (profile → install)`);
      const before = await appVersion(meta.device, app.package);
      if (installCheckFresh(app.id, channel, before)) log(`install ${app.id} "${channel}" skipped: checked in the last ${INSTALL_CHECK_MIN} min, ${before} still installed`);
      else {
        log(`install ${app.id} "${channel}"`);
        if ((await sh(installCmd[0], installCmd.slice(1), { ...env, APP_ID: app.package, CHANNEL: channel }, { cwd: o.appDir })) !== 0) throw new Error(`Installing the latest "${channel}" build failed — see run.log`);
        const after = await appVersion(meta.device, app.package);
        installCheckSave(app.id, channel, after);
        if (restored && after !== before && (await sh(QAPILOT, ['snap', 'save', snap], env, { timeoutMs: 2 * 60_000 })) === 0) snapshotSaved(snap, await appBuild(meta.device, app.package)); // the snapshot follows the new build
      }
    }
    if (o.onboard && !restored && !ios) {
      const onboardCmd = o.onboardCmd || (app.onboarding ? ['maestro', '--device', meta.device, 'test', '--no-reinstall-driver', path.resolve(o.appDir || '.', app.onboarding), '-e', `APP_ID=${app.package}`, '-e', `TEST_USER=${o.user}`] : null);
      if (!onboardCmd) throw new Error(`${meta.appName} has no onboarding flow (profile → onboarding)`);
      log(`onboarding ${o.user}`);
      const ok = await sh(onboardCmd[0], onboardCmd.slice(1), { ...env, APP_ID: app.package, RUN_ID: `${id}-onboarding`, OUT: path.join(dir, 'onboarding') }, { cwd: o.appDir });
      if (ok !== 0) throw new Error('Onboarding failed — see run.log');
      if ((await sh(QAPILOT, ['snap', 'save', snap], env, { timeoutMs: 2 * 60_000 })) === 0) {
        snapshotSaved(snap, await appBuild(meta.device, app.package));
        log(`snapshot ${snap} saved: the next fresh onboarding takes seconds`);
      }
    }
    if (!ios) meta.version = await appVersion(meta.device, app.package);
    if (!meta.version) throw new Error(`${app.package} is not installed on ${meta.device} — install it first (--install)`);
    writeJson(path.join(dir, 'run.json'), meta);

    for (const d of ['evidence', 'flows', 'design']) fs.mkdirSync(path.join(dir, d), { recursive: true });
    const figmaOn = o.claude?.figma !== false;
    const data = home(env);
    writeJson(path.join(dir, 'mcp.json'), {
      mcpServers: {
        // qapilot drives the device (fast screen reads, batches of steps, OTPs); Maestro is kept for flows.
        qapilot: { command: QAPILOT, args: ['mcp'], env: { DEVICE: meta.device, QAPILOT_HOME: data, QAPILOT_JOURNAL: path.join(dir, 'steps.jsonl'), ...(ios ? { QAPILOT_IOS_APPS: meta.package } : {}) } },
        ...(ios ? {} : { maestro: { command: 'maestro', args: ['mcp', '--working-dir', o.appDir || dir], env: { MAESTRO_CLI_NO_ANALYTICS: '1', MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED: 'true' } } }),
        // Same name and URL as a Figma MCP server of your own, so the run reuses its login.
        ...(figmaOn ? { figma: FIGMA_MCP } : {}),
      },
    });
    const figmaMap = path.join(data, 'figma', `${app.id}.json`);
    fs.mkdirSync(path.dirname(figmaMap), { recursive: true });
    env.QAPILOT_JOURNAL = path.join(dir, 'steps.jsonl'); // qapilot from Bash journals too
    const buildNow = ios ? '' : await appBuild(meta.device, app.package); // after an install it is the new build
    const others = ios ? [] : otherUsers(o.users, app, o.user, await snapshots(env), (s) => snapshotMatches(s, buildNow));
    const login = o.login || app.login || DEFAULT_LOGIN;

    // The session id up front: a dashboard can show the conversation live, not only once the agent has finished.
    const sessionId = randomUUID();
    meta.sessions.push(sessionId);
    writeJson(path.join(dir, 'run.json'), meta);
    log(`agent start ${id} (${source.explore ? 'whole app' : source.flow ? 'described flow' : `${meta.issues.length} issue${meta.issues.length > 1 ? 's' : ''}`}, ${meta.appName} ${meta.version})`);
    code = await runClaude(o, dir, claudeArgs(o, dir, meta.device, null, sessionId), `${playbook(meta.device, o.notes)}

---
RUN: ${id}
SOURCE: ${sourceText(source)}
ISSUES:
${meta.issues.length ? meta.issues.map((i) => `- ${i.key}${i.type || i.status ? ` [${[i.type, i.status].filter(Boolean).join(' · ')}]` : ''} ${i.summary || ''} → ${i.file}`).join('\n') : `(none — this run ${source.explore ? 'tests the whole app' : 'checks the FLOW in SOURCE'})`}
FIGMA: ${figmaOn ? (meta.figma.length ? meta.figma.join(' ') : '(no file given — use Figma links found in the issues, else skip the design check)') : 'off'}
FIGMA MAP: ${figmaMap}
FIGMA CACHE: ${path.join(data, 'figma', 'cache')}
FIGMA INDEX: ${figmaMap.replace(/\.json$/, '.index.json')}${fs.existsSync(figmaMap.replace(/\.json$/, '.index.json')) ? '' : ' (not built yet — build it: see the playbook)'}
DESIGN: ${path.join(dir, 'design')}
APP: ${JSON.stringify(app)}
CHANNEL: ${channel || '(none)'}
VERSION: ${meta.version}
DEVICE: ${meta.device}
PLATFORM: ${ios ? `iOS — a real iPhone driven by qapilot's agent (app ${meta.package}). Same qapilot tools and steps; \`back\` is the iOS edge swipe; system pop-ups (permissions, alerts) appear in the screen; no emulator snapshots and no Maestro. Things drawn over the app that are not in the screen tree (a tour's buttons) take \`tap x,y\` from a screenshot.` : 'Android'}
TEST USER: ${o.user}${app.testData?.[o.user] ? ` — ${app.testData[o.user]}` : ''} (its password and PIN are in the environment as TEST_PASSWORD / TEST_PIN — never print them)
LOGIN: ${login}
OTHER USERS: ${others.length ? others.map((u) => `${u.id}${u.data ? ` — ${u.data}` : ''}: \`snapshot\` load ${u.snapshot}, then \`stop ${app.package}\`, \`launch ${app.package}\`, \`digits $TEST_PIN_${u.id.toUpperCase()}\``).join(' · ') : '(none saved for this build — an issue that needs other data is `blocked`: say which data)'}
ONBOARDED: ${ios ? 'the app keeps its login — see LOGIN' : restored ? 'yes — restored from the logged-in snapshot and the app restarted just now: it shows its first screen after a restart (see LOGIN). The emulator clock is already synced — do not touch it' : o.onboard ? 'yes, just now' : 'yes, earlier (check the screen; log in again if the session expired — see LOGIN)'}
NOTE: ${o.note || '(none)'}
TEMPLATE: ${path.join(ROOT, 'templates', 'report.md')}
REPORT: ${path.join(dir, 'report.md')}
EVIDENCE: ${path.join(dir, 'evidence')}
FLOWS: ${path.join(dir, 'flows')}
RESULT: ${resultFile}
This is an unattended run: nobody can answer questions mid-run — use status "needs-input" instead.`, env);
    log(`agent exit ${code}`);
  } catch (e) {
    log('run failed:', e.message);
    writeJson(resultFile, { status: 'failed', summary: e.message, items: [] });
  }
  return finish(dir, meta, code);
}

/** The tester's reply to a run that stopped with "needs-input": the same agent session goes on. */
export async function resume(o) {
  const dir = path.resolve(o.dir);
  const resultFile = path.join(dir, 'result.json');
  const meta = readJson(path.join(dir, 'run.json'), null);
  if (!meta) throw new Error(`No run in ${dir}`);
  meta.pid = process.pid;
  let code = null;
  try {
    const session = meta.sessions?.at(-1);
    if (!session) throw new Error('This run has no agent session to resume');
    // Archive the previous turn so the run reads as "running" until the new result lands.
    const n = fs.readdirSync(dir).filter((f) => /^result-\d+\.json$/.test(f)).length + 1;
    for (const f of ['result', 'claude']) if (fs.existsSync(path.join(dir, `${f}.json`))) fs.renameSync(path.join(dir, `${f}.json`), path.join(dir, `${f}-${n}.json`));
    meta.answers = [...(meta.answers || []), { at: new Date().toISOString(), text: o.answer }];
    writeJson(path.join(dir, 'run.json'), meta);
    const env = await runEnv({ ...o, user: meta.user });
    env.DEVICE = meta.device;
    if (meta.platform === 'ios') env.QAPILOT_IOS_APPS = meta.package;
    env.QAPILOT_JOURNAL = path.join(dir, 'steps.jsonl');
    log(`agent resume ${path.basename(dir)} (session ${session})`);
    code = await runClaude(o, dir, claudeArgs(o, dir, meta.device, session), `The tester replied:

${o.answer}

Continue the mobile QA playbook from where you stopped, with this reply. Same rules as before. Update REPORT and write RESULT again to ${resultFile} with the same JSON shape (all items so far). This is still an unattended run — if you need something else, use status "needs-input" with new questions.`, env);
    log(`agent exit ${code}`);
  } catch (e) {
    log('resume failed:', e.message);
    writeJson(resultFile, { status: 'failed', summary: e.message, items: [] });
  }
  return finish(dir, meta, code);
}
