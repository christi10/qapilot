# Mobile QA report — {{RUN_LABEL}}

| | |
|---|---|
| Source | {{SOURCE}} |
| App | {{APP_NAME}} `{{APP_PACKAGE}}` {{APP_VERSION}} ({{CHANNEL}}) |
| Device | {{DEVICE}} |
| Test user | {{TEST_USER}} |
| Started / finished | {{STARTED}} → {{FINISHED}} |
| Result | {{PASSED}} passed · {{FAILED}} failed · {{BLOCKED}} blocked · {{NOT_TESTABLE}} not testable (a described flow: {{N}} findings) |
| Designs | {{the Figma file(s) compared against, or "no design check"}} |

## Summary

{{SUMMARY — 2-5 sentences: what was tested, what still fails, anything that blocked testing.}}

## Results

{{Issue runs only — leave this section out for a described flow or the whole app.}}

| Issue | Title | Verdict | Confidence | Notes |
|---|---|---|---|---|
{{ONE ROW PER ISSUE: [KEY](url) | summary | ✅ pass / ❌ fail / ⛔ blocked / ➖ not testable | high / medium / low | one line}}

## Details

{{FOR EACH ISSUE:}}
### {{KEY}} — {{TITLE}}

- **Verdict:** {{VERDICT}} · confidence {{high / medium / low — and what it rests on: the engine's number, the element tree, one screenshot}}
- **Tested with:** {{the test user, when it was not the run's TEST USER — and why}}
- **Expected:** {{what the issue / test says should happen}}
- **Actual:** {{what happened on the device}}
- **Steps performed:**
  1. {{step}}
- **Evidence:** {{screenshots as ![caption](evidence/KEY/file.png)}}
- **Replay:** `flows/{{KEY}}.steps` (saved by the run from the steps performed)

## Coverage

{{The whole app only — leave this section out otherwise. One row per screen visited, then the areas and screens not visited.}}

| Area | Screen | Visited | Design | Audit | Findings | Note |
|---|---|---|---|---|---|---|
{{ONE ROW PER SCREEN: area | screen | ✓ / — | checked / no design / not checked | ✓ / — | n | a remark; for a screen not visited, why}}

## Other bugs found

{{EVERY PROBLEM FOUND that is not a tested issue's own verdict — a described flow's whole result, and for issue runs the problems seen on the screens and components around what was tested. Each one written for the developer who fixes it, worst first. None → "None found."}}

| # | Severity | Kind | Bug | Where | Seen while testing |
|---|---|---|---|---|---|
{{ONE ROW PER BUG: ID | severity | kind | title | screen → `component` | [KEY](url) or "—"}}

{{THEN, FOR EACH BUG:}}
### {{ID}} · {{SEVERITY}} · {{KIND}} — {{TITLE}}

| | |
|---|---|
| Where | {{screen}} → `{{component: testID / accessibility label / Figma layer}}` |
| Expected (design) | {{the design's value, with numbers}} |
| Actual (app) | {{what the app does, with numbers}} |
| How to reproduce | {{1. step · 2. step · …}} |
| Suggested fix | {{what to change, as concretely as you can}} |
| Seen while testing | {{[KEY](url) or "free check"}} · [Figma frame]({{figma link}}) |

![FIGMA | APP](design/{{screen}}/Dnn.png)
{{(the engine's picture for a measured bug; else the app screenshot and the Figma render: ![app](evidence/…png) ![design](design/…png))}}

## Design check

{{FOR EACH SCREEN COMPARED WITH FIGMA — paste DESIGN/<screen>/design.md as it is (the engine wrote it: what the design has and what the app shows side by side, the measured differences, the pictures), with its first line changed to:}}
### {{Screen name}} · [Figma {{node id}}]({{frame link}})

{{design.md}}

{{Then 1–3 lines of your own: what the engine does not measure and you checked by eye (texts / language, truncation and mid-word breaks, icons, font weight, states), and which differences are real bugs (each one is in Findings) or explained (live data, an empty state, device width).}}

{{Screens without a matching Figma frame: one line each — "no design".}}

## Environment notes

{{QA-side problems met during the run (OTP not issued, backend errors, first-OTP bug…), with time and what was done.}}
