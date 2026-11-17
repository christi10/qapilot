// What a run leaves behind, checked: result.json as the agent wrote it → the normalised shape, the replay flows cut from
// qapilot's step journal, and the same bug told twice (Android / iOS). Pure functions — no device, no agent.
import fs from 'node:fs';
import path from 'node:path';
import { readJson, writeJson } from './util.mjs';

export const VERDICTS = ['pass', 'fail', 'blocked', 'not-testable'];
export const FINDING_KINDS = ['design', 'functional', 'copy', 'accessibility'];
export const SEVERITIES = ['critical', 'major', 'minor', 'trivial'];
export const CONFIDENCES = ['high', 'medium', 'low'];
export const DESIGN_STATES = ['checked', 'no-design', 'not-checked'];
/** An issue id: a tracker key (PRJ-123), a test case id (TC_12) — letters, digits, - and _, with a digit. */
export const ID_RE = /^(?=.*\d)[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/;
/** What qapilot's `issue` step takes (a described flow marks its steps `issue FLOW`). */
const STEP_ISSUE_RE = /^[A-Za-z0-9_-]{1,40}$/;
/** A Figma design link (design/file/proto), the only kind a run takes. */
export const FIGMA_RE = /^https:\/\/(www\.)?figma\.com\/(design|file|proto)\/[A-Za-z0-9]{22,128}(\/[^\s]*)?$/;
/** An adb serial (emulator-5554, 192.168.1.20:5555) or an iOS UDID. */
export const DEVICE_RE = /^[A-Za-z0-9][A-Za-z0-9._:-]{2,63}$/;
/** iPhone (00008110-0012345678ABCDEF) or simulator (8-4-4-4-12) UDIDs — the same rule as qapilot's. */
export const IOS_UDID_RE = /^([0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}|[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}|[0-9A-Fa-f]{40})$/;

/** A path inside a run folder: relative, no `..`, no absolute. */
export function safeRel(f) {
  return typeof f === 'string' && f.length < 300 && !path.isAbsolute(f) && !f.split(/[\\/]/).includes('..') && /^[\w./ -]+$/.test(f);
}

/** result.json as a dashboard can show it, whatever the agent wrote. */
export function normalizeResult(r) {
  if (!r || typeof r !== 'object') return null;
  const items = (Array.isArray(r.items) ? r.items : []).filter((i) => i && ID_RE.test(String(i.key))).map((i) => ({
    key: String(i.key),
    title: String(i.title || ''),
    verdict: VERDICTS.includes(i.verdict) ? i.verdict : 'blocked',
    notes: String(i.notes || ''),
    evidence: (Array.isArray(i.evidence) ? i.evidence : []).map(String).filter((f) => safeRel(f)),
    flow: i.flow && safeRel(String(i.flow)) ? String(i.flow) : '',
    flowPasses: i.flowPasses === true,
    // how sure the verdict is: measured / asserted (high), seen once (medium), inferred (low); '' when the run did not say
    confidence: CONFIDENCES.includes(i.confidence) ? i.confidence : '',
  }));
  return {
    status: ['done', 'needs-input', 'failed', 'cancelled'].includes(r.status) ? r.status : 'failed',
    summary: String(r.summary || ''),
    items,
    questions: (Array.isArray(r.questions) ? r.questions : []).map(String),
    findings: (Array.isArray(r.findings) ? r.findings : []).filter((f) => f && f.title).slice(0, 100).map((f, n) => ({
      id: String(f.id || `F${n + 1}`).slice(0, 20),
      title: String(f.title).slice(0, 300),
      kind: FINDING_KINDS.includes(f.kind) ? f.kind : 'functional',
      severity: SEVERITIES.includes(f.severity) ? f.severity : 'minor',
      screen: String(f.screen || ''),
      component: String(f.component || '').slice(0, 300),
      issue: ID_RE.test(String(f.issue)) ? String(f.issue) : '',
      steps: (Array.isArray(f.steps) ? f.steps : []).map(String),
      expected: String(f.expected || ''),
      actual: String(f.actual || ''),
      evidence: (Array.isArray(f.evidence) ? f.evidence : []).map(String).filter((x) => safeRel(x)),
      design: f.design && safeRel(String(f.design)) ? String(f.design) : '',
      figma: FIGMA_RE.test(String(f.figma)) ? String(f.figma) : '',
      fix: String(f.fix || '').slice(0, 1000),
    })),
    // A whole-app run: every screen it visited (and the areas it could not reach), so "tested the whole app" can be checked.
    coverage: (Array.isArray(r.coverage) ? r.coverage : []).filter((c) => c && (c.screen || c.area)).slice(0, 300).map((c) => {
      const row = {
        area: String(c.area || '').slice(0, 100),
        screen: String(c.screen || '').slice(0, 150),
        design: DESIGN_STATES.includes(c.design) ? c.design : 'not-checked',
        audit: c.audit === true,
        findings: Number.isInteger(c.findings) && c.findings > 0 ? c.findings : 0,
        note: String(c.note || '').slice(0, 300),
      };
      // Visited or not: what the run said; a row without it was visited unless nothing at all was done on it and a
      // note says why (a visited screen has a note too — "stopped on the review sheet").
      const untouched = !row.screen || (!!row.note && !row.audit && row.design === 'not-checked' && !row.findings);
      return { ...row, reached: typeof c.reached === 'boolean' ? c.reached : !untouched };
    }),
  };
}

/**
 * The replay of each issue, cut from the journal qapilot keeps of every step that worked (QAPILOT_JOURNAL): the steps
 * after `issue <KEY>` up to the next one. Screenshots and screen names are left out — a replay does not need them.
 */
export function replayFlows(journal) {
  const flows = new Map();
  let key = '';
  for (const line of String(journal).split('\n')) {
    let e;
    try {
      e = JSON.parse(line);
    } catch {
      continue;
    }
    if (typeof e.issue === 'string') key = STEP_ISSUE_RE.test(e.issue) ? e.issue.toUpperCase() : '';
    else if (key && typeof e.step === 'string' && !/^(shot|name)\b/.test(e.step)) flows.set(key, [...(flows.get(key) || []), e.step]);
  }
  return flows;
}

/** Writes flows/<KEY>.steps from the journal and points each issue without a flow of its own at it. */
export function saveFlows(dir, meta) {
  const file = path.join(dir, 'steps.jsonl');
  if (!fs.existsSync(file)) return 0;
  const flows = replayFlows(fs.readFileSync(file, 'utf8'));
  const titles = new Map((meta.issues || []).map((i) => [i.key, i.summary]));
  for (const [key, steps] of flows) {
    const out = path.join(dir, 'flows', `${key}.steps`);
    fs.mkdirSync(path.dirname(out), { recursive: true });
    fs.writeFileSync(out, [
      `# ${key}${titles.get(key) ? ` — ${titles.get(key)}` : ''}`,
      `# ${meta.appName} ${meta.version} (${meta.package}) · user ${meta.user} · ${meta.device}`,
      `# The steps the run performed, as they replay (start on Home, logged in): qapilot do --file ${out}`,
      ...steps, '',
    ].join('\n'));
  }
  const resultFile = path.join(dir, 'result.json');
  const result = readJson(resultFile, null);
  if (result && Array.isArray(result.items)) {
    let changed = false;
    for (const i of result.items) {
      if (i && !i.flow && flows.has(String(i.key))) {
        i.flow = `flows/${i.key}.steps`;
        changed = true;
      }
    }
    if (changed) writeJson(resultFile, result);
  }
  return flows.size;
}

const STOP = new Set(['ios', 'android', 'the', 'and', 'with', 'instead', 'from', 'that', 'this', 'του', 'της', 'και', 'στο', 'στην', 'των']);
const words = (s, stop) => new Set(String(s).toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').split(/[^\p{L}\p{N}]+/u).filter((w) => w.length >= 3 && !STOP.has(w) && !stop.has(w)));

/**
 * The same bug reported by the Android run and by the iOS run in other words: same kind and screen, and most words
 * shared. `appWords` (the app's name, its brand) are left out of the comparison — every title has them.
 */
export function sameBug(x, y, appWords = []) {
  if (x.kind !== y.kind || String(x.screen).toLowerCase() !== String(y.screen).toLowerCase()) return false;
  const stop = new Set(appWords.flatMap((w) => String(w).toLowerCase().split(/[^\p{L}\p{N}]+/u)));
  const [a, b] = [words(`${x.title} ${x.component}`, stop), words(`${y.title} ${y.component}`, stop)];
  const common = [...a].filter((w) => b.has(w)).length;
  return common / (a.size + b.size - common || 1) >= 0.3;
}

/** android | ios, from the run's own record (older runs: an iOS UDID as the device). */
export const runPlatform = (meta) => meta.platform || (IOS_UDID_RE.test(meta.device || '') ? 'ios' : 'android');
