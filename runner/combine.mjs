// The same issues tested on Android and on iOS, in one report and one run: the verdicts side by side, every screen both
// checked against the same Figma frame as DESIGN | ANDROID | iOS (qapilot cross), and every other bug with its platform.
// Built from the two finished runs' files — no device, no agent.
import fs from 'node:fs';
import path from 'node:path';
import { QAPILOT, readJson, writeJson, exec } from './util.mjs';
import { SEVERITIES, runPlatform, sameBug } from './result.mjs';

const VERDICT_ICON = { pass: '✅ pass', fail: '❌ fail', blocked: '⛔ blocked', 'not-testable': '➖ not testable' };
const VERDICT_RANK = ['fail', 'blocked', 'not-testable', 'pass'];
const RUN_RE = /^[\w.-]{1,120}$/;

/** The design checks of a run by Figma frame: node id → { screen, dir }, those with the inputs a cross check needs. */
function designChecks(dir) {
  const out = new Map();
  const base = path.join(dir, 'design');
  if (!fs.existsSync(base)) return out;
  for (const s of fs.readdirSync(base)) {
    const inp = readJson(path.join(base, s, 'inputs.json'), null);
    if (inp?.figma_name && fs.existsSync(path.join(base, s, 'app-boxes.json')) && !out.has(inp.figma_name)) out.set(inp.figma_name, { screen: s, dir: path.join(base, s) });
  }
  return out;
}

/** A finding's file paths moved under android/ or ios/ (where combine copies them). */
const under = (sub, f) => (f ? `${sub}/${f}` : '');

/**
 * Combines two finished runs (one Android, one iOS) under `runsDir` into a CROSS-<time> run; returns its id.
 *   issueUrl  'https://tracker.example/browse/{key}' — issue keys become links in the report
 *   runLink   '#run:{id}' — the two source runs become links (e.g. into a dashboard)
 */
export async function combine(runsDir, a, b, { issueUrl = '', runLink = '' } = {}) {
  const load = (id) => {
    if (!RUN_RE.test(String(id || '')) || !fs.existsSync(path.join(runsDir, id, 'run.json'))) throw new Error(`usage: combine <android run> <ios run> — no such run: ${id}`);
    const dir = path.join(runsDir, id);
    return { id, dir, meta: readJson(path.join(dir, 'run.json'), {}), result: readJson(path.join(dir, 'result.json'), null) };
  };
  const runs = [load(a), load(b)];
  const android = runs.find((r) => runPlatform(r.meta) === 'android');
  const ios = runs.find((r) => runPlatform(r.meta) === 'ios');
  if (!android || !ios) throw new Error('combine needs one Android run and one iOS run');
  if (!android.result || !ios.result) throw new Error('both runs must have finished (result.json)');
  for (const r of [android, ios]) r.result = { items: [], findings: [], ...r.result, findings: (r.result.findings || []).map((f) => ({ evidence: [], steps: [], ...f })) };
  const id = `CROSS-${new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19)}`;
  const dir = path.join(runsDir, id);
  fs.mkdirSync(dir, { recursive: true });
  const issues = [...new Map([...android.meta.issues || [], ...ios.meta.issues || []].map((i) => [i.key, i])).values()];
  const now = new Date().toISOString();
  const meta = {
    label: 'CROSS', platform: 'cross', runs: { android: android.id, ios: ios.id }, app: android.meta.app, appName: android.meta.appName,
    package: `${android.meta.package} · ${ios.meta.package}`, channel: android.meta.channel, user: android.meta.user,
    version: android.meta.version === ios.meta.version ? android.meta.version : `${android.meta.version} / ${ios.meta.version}`,
    device: `${android.meta.device} · ${ios.meta.device}`, source: { exec: null, jql: null, issues: issues.map((i) => i.key), flow: null },
    issues, sessions: [], answers: [], started: now, figma: [...new Set([...(android.meta.figma || []), ...(ios.meta.figma || [])])],
  };
  writeJson(path.join(dir, 'run.json'), meta); // first: qapilot cross writes its links relative to this folder
  // Both runs' pictures, so the report finds them inside this run.
  for (const [r, sub] of [[android, 'android'], [ios, 'ios']]) {
    for (const d of ['evidence', 'design']) if (fs.existsSync(path.join(r.dir, d))) fs.cpSync(path.join(r.dir, d), path.join(dir, sub, d), { recursive: true });
  }
  // Every screen both runs checked against the same Figma frame.
  const [ca, ci] = [designChecks(android.dir), designChecks(ios.dir)];
  const sections = [];
  for (const [node, x] of ca) {
    const y = ci.get(node);
    if (!y) continue;
    const out = path.join(dir, 'design', x.screen);
    const r = await exec(QAPILOT, ['cross', '--android', x.dir, '--ios', y.dir, '--out', out], { timeout: 120_000 });
    if (r.err) sections.push(`### ${x.screen} · Figma ${node}\n\nThe cross-platform check failed: ${(r.stderr || r.err.message || '').trim()}\n`);
    else sections.push(fs.readFileSync(path.join(out, 'cross.md'), 'utf8').replace(/^#### Design check · Figma ([^\n]*)/, `### ${x.screen} · Figma $1`));
  }
  const onlyOne = [...ca.keys()].filter((n) => !ci.has(n)).map((n) => `${ca.get(n).screen} (Figma ${n}): Android only`)
    .concat([...ci.keys()].filter((n) => !ca.has(n)).map((n) => `${ci.get(n).screen} (Figma ${n}): iOS only`));

  // Verdicts side by side; the combined verdict is the worse of the two.
  const verdict = (r, key) => r.result.items.find((i) => i.key === key);
  const items = issues.map((i) => {
    const [va, vi] = [verdict(android, i.key), verdict(ios, i.key)];
    const worst = [va?.verdict, vi?.verdict].filter(Boolean).sort((x, y) => VERDICT_RANK.indexOf(x) - VERDICT_RANK.indexOf(y))[0] || 'not-testable';
    return {
      key: i.key, title: i.summary || va?.title || vi?.title || '', verdict: worst,
      notes: `Android: ${va?.verdict || '—'}${va?.notes ? ` — ${va.notes}` : ''} · iOS: ${vi?.verdict || '—'}${vi?.notes ? ` — ${vi.notes}` : ''}`,
      evidence: [...(va?.evidence || []).map((e) => under('android', e)), ...(vi?.evidence || []).map((e) => under('ios', e))],
      android: va?.verdict || '', ios: vi?.verdict || '',
    };
  });
  const findings = [
    ...android.result.findings.map((f) => ({ ...f, id: `A-${f.id}`, title: `[Android] ${f.title}`, evidence: f.evidence.map((e) => under('android', e)), design: under('android', f.design), platform: 'Android' })),
    ...ios.result.findings.map((f) => ({ ...f, id: `I-${f.id}`, title: `[iOS] ${f.title}`, evidence: f.evidence.map((e) => under('ios', e)), design: under('ios', f.design), platform: 'iOS' })),
  ];
  const sev = (f) => SEVERITIES.indexOf(f.severity);
  findings.sort((x, y) => sev(x) - sev(y));
  // One entry per bug: an iOS finding that is the same bug as an Android one joins it ("Android + iOS").
  const appWords = [meta.appName, meta.app].filter(Boolean);
  const bugs = [];
  for (const f of findings) {
    const twin = f.platform === 'iOS' && bugs.find((b) => b.platform === 'Android' && !b.twin && sameBug(b, f, appWords));
    if (twin) twin.twin = f;
    else bugs.push({ ...f });
  }
  const count = (pf) => VERDICT_RANK.map((v) => `${items.filter((i) => i[pf] === v).length} ${v}`).join(' · ');

  // The report.
  const link = (k) => (issueUrl ? `[${k}](${issueUrl.replaceAll('{key}', k)})` : k);
  const runRef = (r) => (runLink ? `[${r}](${runLink.replaceAll('{id}', r)})` : r);
  const md = [];
  md.push(`# Mobile QA report — Android and iOS · ${meta.appName} ${meta.version}`, '');
  md.push('| | Android | iOS |', '|---|---|---|');
  md.push(`| Run | ${runRef(android.id)} | ${runRef(ios.id)} |`);
  md.push(`| App | ${android.meta.appName} ${android.meta.version} \`${android.meta.package}\` | ${ios.meta.appName} ${ios.meta.version} \`${ios.meta.package}\` |`);
  md.push(`| Device | ${android.meta.device} | ${ios.meta.device} |`);
  md.push(`| Test user | ${android.meta.user} | ${ios.meta.user} |`);
  md.push(`| Result | ${count('android')} | ${count('ios')} |`, '');
  const same = items.filter((i) => i.android && i.android === i.ios);
  const diff = items.filter((i) => i.android && i.ios && i.android !== i.ios);
  md.push('## Summary', '');
  const both = bugs.filter((b) => b.twin).length;
  md.push(`${items.length} issue(s) on both platforms: ${same.length} with the same verdict${same.length ? ` (${same.map((i) => `${i.key} ${i.android}`).join(', ')})` : ''}${diff.length ? `; different on ${diff.map((i) => `${i.key} (Android ${i.android}, iOS ${i.ios})`).join(', ')}` : ''}. ${bugs.length} other bug(s): ${both} on both platforms, ${bugs.filter((b) => !b.twin && b.platform === 'Android').length} on Android only, ${bugs.filter((b) => !b.twin && b.platform === 'iOS').length} on iOS only. ${sections.length} screen(s) compared with Figma on both.`, '');
  md.push('## Results', '', '| Issue | Title | Android | iOS | Notes |', '|---|---|---|---|---|');
  for (const i of items) {
    const [va, vi] = [verdict(android, i.key), verdict(ios, i.key)];
    md.push(`| ${link(i.key)} | ${(i.title || '').replace(/\|/g, '/')} | ${VERDICT_ICON[i.android] || '—'} | ${VERDICT_ICON[i.ios] || '—'} | ${`Android: ${va?.notes || '—'} · iOS: ${vi?.notes || '—'}`.replace(/\|/g, '/').replace(/\n/g, ' ')} |`);
  }
  md.push('', '## Design check — Design | Android | iOS', '');
  if (sections.length) md.push(sections.join('\n'));
  else md.push('No screen was checked against the same Figma frame on both platforms (each run needs a design check of the screen, with the same screen folder and Figma frame).', '');
  if (onlyOne.length) md.push('', `Checked on one platform only: ${onlyOne.join(' · ')} — see that run's report.`, '');
  md.push('', '## Other bugs found', '');
  const where = (f) => `${(f.screen || '—').replace(/\|/g, '/')}${f.component ? ' → `' + f.component + '`' : ''}`;
  const plat = (b) => (b.twin ? 'Android + iOS' : b.platform);
  const bare = (t) => t.replace(/^\[(Android|iOS)\] /, '').replace(/\|/g, '/');
  if (!bugs.length) md.push('None found.');
  else {
    md.push('| # | Platform | Severity | Kind | Bug | Where | Seen while testing |', '|---|---|---|---|---|---|---|');
    for (const b of bugs) md.push(`| ${b.id}${b.twin ? ' + ' + b.twin.id : ''} | ${plat(b)} | ${b.severity} | ${b.kind} | ${bare(b.title)} | ${where(b)} | ${b.issue ? link(b.issue) : '—'} |`);
    for (const b of bugs) {
      md.push('', `### ${b.id}${b.twin ? ' + ' + b.twin.id : ''} · ${plat(b)} · ${b.severity} · ${b.kind} — ${bare(b.title)}`, '', '| | |', '|---|---|');
      md.push(`| Where | ${where(b)} |`);
      md.push(`| Expected (design) | ${(b.expected || '—').replace(/\|/g, '/')} |`);
      md.push(`| Actual (app) | ${b.twin ? '**Android:** ' : ''}${(b.actual || '—').replace(/\|/g, '/')}${b.twin ? ' · **iOS:** ' + (b.twin.actual || '—').replace(/\|/g, '/') : ''} |`);
      md.push(`| How to reproduce | ${b.steps.length ? b.steps.map((x, n) => (n + 1) + '. ' + x).join(' · ').replace(/\|/g, '/') : '—'} |`);
      md.push(`| Suggested fix | ${(b.fix || b.twin?.fix || '—').replace(/\|/g, '/')} |`);
      md.push(`| Seen while testing | ${b.issue ? link(b.issue) : 'free check'}${b.figma ? ' · [Figma frame](' + b.figma + ')' : ''} |`, '');
      for (const f of [b, b.twin].filter(Boolean)) {
        if (f.design) md.push(`![${f.platform} — ${/\/D\d+\.png$/.test(f.design) ? 'FIGMA | APP' : 'design'}](${f.design})`);
        for (const e of f.evidence.slice(0, 1)) md.push(`![${f.platform} — app](${e})`);
      }
    }
  }
  fs.writeFileSync(path.join(dir, 'report.md'), md.join('\n') + '\n');
  writeJson(path.join(dir, 'result.json'), {
    status: 'done',
    summary: md[md.indexOf('## Summary') + 2] || '',
    items,
    findings: findings.map(({ platform, ...f }) => f),
  });
  meta.finished = new Date().toISOString();
  writeJson(path.join(dir, 'run.json'), meta);
  console.log(`combine ${id}: ${items.length} issue(s), ${sections.length} screen(s) compared on both, ${findings.length} other bug(s)\n  report: ${path.join(dir, 'report.md')}`);
  return id;
}
