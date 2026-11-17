import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import * as q from '../runner/index.mjs';
import { parseArgs, issueFiles } from '../runner/cli.mjs';

const tmp = (p) => fs.mkdtempSync(path.join(os.tmpdir(), p));

test('run ids, labels and issue files', () => {
  assert.equal(q.runLabel({ issues: [{ key: 'PRJ-1' }] }), 'PRJ-1');
  assert.equal(q.runLabel({ issues: [{ key: 'PRJ-1' }, { key: 'PRJ-2' }] }), 'ISSUES');
  assert.equal(q.runLabel({ flow: 'Open the cart' }), 'FLOW');
  assert.equal(q.runLabel({ explore: true }), 'APP');
  assert.equal(q.runLabel({ label: 'JQL', issues: [] }), 'JQL');
  assert.equal(q.runId('PRJ-1', new Date(Date.UTC(2026, 10, 17, 9, 30, 5))), 'PRJ-1-2026-11-17T09-30-05');

  const dir = tmp('qapilot-issues-');
  fs.writeFileSync(path.join(dir, 'a.md'), '# prj-7: Pay button is too low\n\n- Type: Bug · Status: Ready for Test · Priority: High\n\nThe gap should be 32 dp.\n');
  fs.mkdirSync(path.join(dir, 'TC_12'));
  fs.writeFileSync(path.join(dir, 'TC_12', 'issue.md'), 'Log in and open the cart.\n');
  fs.writeFileSync(path.join(dir, 'notes.txt'), 'not an issue');
  const issues = issueFiles({ issues: [], issueDir: dir });
  assert.deepEqual(issues.map(({ key, summary, type, status }) => [key, summary, type, status]), [['TC_12', '', '', ''], ['PRJ-7', 'Pay button is too low', 'Bug', 'Ready for Test']]); // folder order
  assert.ok(path.isAbsolute(issues[1].file));
  fs.writeFileSync(path.join(dir, 'bad.md'), 'no heading here');
  assert.throws(() => q.readIssue(path.join(dir, 'bad.md')), /no issue id/);
});

test('the CLI checks what a run gets', () => {
  const a = parseArgs(['run', '--app', 'apps/shop.json', '--user', 'u1', '--issue', 'a.md', '--issue', 'b.md', '--install']);
  assert.deepEqual([a.app, a.user, a.issues, a.install, a.onboard], ['apps/shop.json', 'u1', ['a.md', 'b.md'], true, false]);
  assert.throws(() => parseArgs(['run', '--app', 'x', '--user', '1', '--flow', 'a', '--issue', 'a.md']), /usage/); // one source
  assert.throws(() => parseArgs(['run', '--app', 'x', '--user', '1']), /usage/);
  const e = parseArgs(['run', '--app', 'x', '--user', '1', '--explore', 'Checkout first', '--install']);
  assert.deepEqual([e.explore, e.focus, e.install], [true, 'Checkout first', true]);
  const bare = parseArgs(['run', '--app', 'x', '--user', '1', '--explore', '--install']); // the next flag is not a focus
  assert.deepEqual([bare.explore, bare.focus, bare.install], [true, undefined, true]);
  assert.equal(parseArgs(['run', '--app', 'x', '--user', '1', '--flow', 'a', '--device', '00008110-0012345678ABCDEF']).device, '00008110-0012345678ABCDEF');
  assert.throws(() => parseArgs(['run', '--app', 'x', '--user', '1', '--flow', 'a', '--device', 'a; rm -rf /']), /Not a device/);
  assert.throws(() => parseArgs(['run', '--app', 'x', '--user', '1', '--flow', 'a', '--figma', 'https://example.com/design/abc']), /Not a Figma design link/);
  assert.ok(parseArgs(['run', '--app', 'x', '--user', '1', '--flow', 'a', '--figma', 'https://www.figma.com/design/AbCdEfGhIjKlMnOpQrStUv/?node-id=45-5012']));
  assert.throws(() => parseArgs(['resume', 'RUN']), /usage/); // an answer is needed
  assert.deepEqual(parseArgs(['combine', 'A', 'B', '--issue-url', 'https://t/{key}']).pos, ['A', 'B']);
  assert.throws(() => parseArgs(['run', '--app']), /needs a value/);
  assert.ok(q.IOS_UDID_RE.test('00008110-0012345678ABCDEF') && q.IOS_UDID_RE.test('60E1D21F-1649-4D55-ABFB-84E7960019B6'));
  assert.ok(!q.IOS_UDID_RE.test('emulator-5554'));
});

test('result.json is normalised and its paths stay inside the run folder', () => {
  for (const ok of ['evidence/PRJ-1/01-home.png', 'flows/PRJ-1.yaml', 'report.md']) assert.ok(q.safeRel(ok), ok);
  for (const bad of ['../x.png', '/etc/passwd', 'evidence/../../x', 'a\\..\\b', 'x;rm.png']) assert.ok(!q.safeRel(bad), bad);
  const r = q.normalizeResult({
    status: 'done',
    items: [
      { key: 'PRJ-1', verdict: 'pass', evidence: ['evidence/PRJ-1/01.png', '../../secret.png'], flow: 'flows/PRJ-1.yaml', flowPasses: true, confidence: 'high' },
      { key: 'PRJ-2', verdict: 'maybe', confidence: 'certain' },
      { key: 'not a key', verdict: 'pass' },
      { key: 'NOKEY', verdict: 'pass' }, // an id has a digit
    ],
  });
  assert.equal(r.status, 'done');
  assert.deepEqual(r.items.map((i) => [i.key, i.verdict, i.confidence]), [['PRJ-1', 'pass', 'high'], ['PRJ-2', 'blocked', '']]);
  assert.deepEqual(r.items[0].evidence, ['evidence/PRJ-1/01.png']);
  assert.equal(q.normalizeResult({ status: 'weird' }).status, 'failed');
  assert.equal(q.normalizeResult(null), null);
  const withFindings = q.normalizeResult({
    status: 'done',
    findings: [
      { title: 'Pay button is green', kind: 'design', severity: 'minor', evidence: ['evidence/FLOW/03.png', '/etc/x.png'], design: 'design/cart.png', figma: 'https://www.figma.com/design/AbCdEfGhIjKlMnOpQrStUv/?node-id=1-2', issue: 'nope' },
      { title: 'Crash on Pay', kind: 'weird', severity: 'blocker', design: '../x.png', figma: 'https://evil.example/x', issue: 'PRJ-3' },
      { kind: 'design' },
    ],
  });
  assert.deepEqual(withFindings.findings.map((f) => [f.id, f.kind, f.severity, f.evidence.length, f.design, !!f.figma, f.issue]), [
    ['F1', 'design', 'minor', 1, 'design/cart.png', true, ''],
    ['F2', 'functional', 'minor', 0, '', false, 'PRJ-3'],
  ]);
  const cov = q.normalizeResult({
    status: 'done',
    coverage: [
      { area: 'Shop', screen: 'Cart', design: 'checked', audit: true, findings: 2 },
      { area: 'Cards', screen: '', design: 'weird', findings: -1, note: 'needs a card no test user has' },
      { design: 'checked' },
      { area: 'Shop', screen: 'Review', design: 'no-design', note: 'stopped here' }, // a note alone does not mean "not reached"
      { area: 'Shop', screen: 'Gift cards', note: 'not reached in this run' },
      { area: 'Shop', screen: 'Help', reached: false, design: 'no-design' },
    ],
  });
  assert.deepEqual(cov.coverage.slice(0, 2), [
    { area: 'Shop', screen: 'Cart', design: 'checked', audit: true, findings: 2, note: '', reached: true },
    { area: 'Cards', screen: '', design: 'not-checked', audit: false, findings: 0, note: 'needs a card no test user has', reached: false },
  ]);
  assert.deepEqual(cov.coverage.map((c) => c.reached), [true, false, true, false, false]);
});

test('replay flows are cut from the journal of the steps that worked', () => {
  const journal = [
    { at: 1, step: 'digits $TEST_PIN', ms: 900, screen: 'aa' }, // before any issue: the login, nobody's flow
    { at: 2, issue: 'PRJ-1' },
    { at: 3, step: 'goto Home', ms: 300, screen: 'bb' },
    { at: 4, step: 'tap id:cart-tab', ms: 400, screen: 'cc' },
    { at: 5, step: 'shot /runs/x/evidence/PRJ-1/01-cart.png', ms: 200, screen: 'cc' },
    { at: 6, step: 'name Cart', ms: 1, screen: 'cc' },
    { at: 7, issue: 'prj-2' },
    { at: 8, step: 'wait:30 Το καλάθι σου', ms: 1200, screen: 'cc' },
    { at: 9, issue: '../etc' }, // not an id: what follows belongs to nobody
    { at: 10, step: 'back', ms: 100, screen: 'bb' },
    { at: 11, issue: 'FLOW' },
    { at: 12, step: 'tap Checkout', ms: 100, screen: 'dd' },
  ].map((e) => JSON.stringify(e)).join('\n') + '\nnot json\n';
  assert.deepEqual([...q.replayFlows(journal)], [['PRJ-1', ['goto Home', 'tap id:cart-tab']], ['PRJ-2', ['wait:30 Το καλάθι σου']], ['FLOW', ['tap Checkout']]]);

  const dir = tmp('qapilot-flows-');
  fs.writeFileSync(path.join(dir, 'steps.jsonl'), journal);
  fs.writeFileSync(path.join(dir, 'result.json'), JSON.stringify({ status: 'done', items: [{ key: 'PRJ-1', verdict: 'pass' }, { key: 'PRJ-2', verdict: 'fail', flow: 'flows/PRJ-2.yaml' }, { key: 'PRJ-3', verdict: 'blocked' }] }));
  const meta = { appName: 'Shop', version: '2.3.0', package: 'com.example.shop', user: 'u1', device: 'emulator-5554', issues: [{ key: 'PRJ-1', summary: 'Cart gap' }] };
  assert.equal(q.saveFlows(dir, meta), 3);
  const steps = fs.readFileSync(path.join(dir, 'flows', 'PRJ-1.steps'), 'utf8').split('\n');
  assert.equal(steps[0], '# PRJ-1 — Cart gap');
  assert.deepEqual(steps.filter((l) => l && !l.startsWith('#')), ['goto Home', 'tap id:cart-tab']);
  // an issue gets the saved steps as its flow unless the run named one itself; an issue with no steps gets none
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(dir, 'result.json'), 'utf8')).items.map((i) => i.flow), ['flows/PRJ-1.steps', 'flows/PRJ-2.yaml', undefined]);
  assert.equal(q.saveFlows(tmp('qapilot-none-'), meta), 0); // no journal: nothing to do
});

test('the same bug told by the Android run and by the iOS run', () => {
  const a = { kind: 'design', screen: 'Cart', title: 'Shop: Cart tab strip cuts the «Αρχική» tab off at the left screen edge', component: 'Nav-Tabs / top tab strip' };
  const i = { kind: 'design', screen: 'Cart', title: 'Shop iOS: Cart tab strip opens scrolled, clipping "Αρχική" at the left edge', component: 'Nav-Tabs (tab "Αρχική")' };
  assert.ok(q.sameBug(a, i, ['Shop']));
  assert.ok(!q.sameBug(a, { ...i, kind: 'copy' }));
  assert.ok(!q.sameBug(a, { kind: 'design', screen: 'Cart', title: 'Action labels break mid-word', component: 'Action bar labels' }, ['Shop']));
});

test('other test users, snapshot names and the 30-minute install check', () => {
  const app = { id: 'shop', testData: { u2: 'has saved cards', u1: 'no cards' } };
  const users = [{ id: 'u1', app: 'shop' }, { id: 'u2', app: 'shop', label: 'second user' }, { id: 'u3', app: 'other' }, { id: 'u4', app: 'shop' }];
  const saved = new Set(['shop-u1', 'shop-u2', 'other-u3']);
  assert.deepEqual(q.otherUsers(users, app, 'u1', saved, () => true), [{ id: 'u2', snapshot: 'shop-u2', data: 'has saved cards' }]);
  assert.deepEqual(q.otherUsers(users, app, 'u1', saved, () => false), []); // saved with another build: not usable
  assert.deepEqual(q.otherUsers(undefined, app, 'u1', saved, () => true), []);
  assert.equal(q.snapshotName('shop', 'a b/c'), 'shop-a_b_c');
  const file = path.join(tmp('qapilot-'), 'install-check.json');
  const now = Date.UTC(2026, 10, 9, 18, 0);
  fs.writeFileSync(file, JSON.stringify({ 'shop|QA': { at: now - 10 * 60_000, version: '2.3.0' } }));
  assert.ok(q.installCheckFresh('shop', 'QA', '2.3.0', now, file));
  assert.ok(!q.installCheckFresh('shop', 'QA', '2.3.1', now, file)); // another build is installed
  assert.ok(!q.installCheckFresh('shop', 'QA', '2.3.0', now + 25 * 60_000, file)); // older than 30 min
  assert.ok(!q.installCheckFresh('shop', 'Beta', '2.3.0', now, file)); // another channel
});

test('combine: one Android and one iOS run into a CROSS run', async () => {
  const runs = tmp('qapilot-runs-');
  const mk = (id, meta, result) => {
    fs.mkdirSync(path.join(runs, id), { recursive: true });
    fs.writeFileSync(path.join(runs, id, 'run.json'), JSON.stringify(meta));
    fs.writeFileSync(path.join(runs, id, 'result.json'), JSON.stringify(result));
  };
  const base = { app: 'shop', appName: 'Shop', user: 'u1', version: '2.3.0', issues: [{ key: 'PRJ-1', summary: 'Cart gap' }] };
  mk('A', { ...base, platform: 'android', device: 'emulator-5554', package: 'com.example.shop' }, { status: 'done', items: [{ key: 'PRJ-1', verdict: 'pass' }], findings: [{ id: 'F1', title: 'Shop: Pay label breaks mid-word', kind: 'copy', severity: 'minor', screen: 'Cart' }] });
  mk('B', { ...base, device: '00008110-0012345678ABCDEF', package: 'com.example.shop' }, { status: 'done', items: [{ key: 'PRJ-1', verdict: 'fail', notes: 'still 36 dp' }], findings: [] });
  const log = console.log;
  console.log = () => {};
  const id = await q.combine(runs, 'A', 'B', { issueUrl: 'https://tracker.example/browse/{key}', runLink: '#run:{id}' }).finally(() => (console.log = log));
  const report = fs.readFileSync(path.join(runs, id, 'report.md'), 'utf8');
  assert.match(report, /\[PRJ-1\]\(https:\/\/tracker\.example\/browse\/PRJ-1\)/);
  assert.match(report, /\[A\]\(#run:A\)/);
  const result = JSON.parse(fs.readFileSync(path.join(runs, id, 'result.json'), 'utf8'));
  assert.deepEqual(result.items.map((i) => [i.key, i.verdict, i.android, i.ios]), [['PRJ-1', 'fail', 'pass', 'fail']]); // the worse verdict
  assert.deepEqual(result.findings.map((f) => f.id), ['A-F1']);
  await assert.rejects(q.combine(runs, 'A', '../etc'), /no such run/);
  await assert.rejects(q.combine(runs, 'A', 'A'), /one Android run and one iOS run/);
});
