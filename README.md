# qapilot

Fast Android and iOS device control for testing mobile apps — by hand, from scripts, or by an AI agent over MCP — and
an unattended AI tester that uses it: give it bugs to re-test, a flow in plain words or the whole app, and it drives a
real device the way a careful tester would, then writes a report with screenshots, a verdict per issue, a
measured design check against Figma and a bug report per problem it found.

- **Driver** (Rust, one binary): reads a screen in ~0.1 s (works on animated screens), taps in ~20 ms, runs a whole
  intent in one call, waits for the screen to settle by itself, learns the paths between screens (`goto Cart`).
  Android emulator or phone over adb and Maestro's on-device driver (no JVM per call); iPhone and iOS simulator
  through its own XCTest agent.
- **Design check**: the live screen against its Figma frame, measured in dp in under a second — gaps, heights,
  margins, content cut off at the edge, text colours (CIEDE2000) — with a FIGMA | APP picture per difference.
  `cross` puts the same frame on Android and iOS side by side.
- **Audit**: tree checks of a screen in milliseconds, the same answer every run — values that never reached the
  screen (`undefined`, `{{name}}`, translation keys), unlabelled or tiny tap targets, clipped or truncated text,
  text in the other language, overlapping tap targets.
- **Runner** (Node, no dependencies): starts [Claude Code](https://claude.com/claude-code) with the driver, Maestro
  and Figma as MCP servers and a tester's playbook, unattended, and collects `report.md` + `result.json`.

## Requirements

macOS (the iOS parts need Xcode; the Android parts need the Android SDK), and:

| | For | Install |
|---|---|---|
| Rust (cargo) | building the driver (the launcher builds it on first use) | https://rustup.rs |
| Android SDK platform-tools + an emulator, Maestro CLI | Android (qapilot installs Maestro's on-device driver from your Maestro) | Android Studio · `brew install maestro` |
| Xcode, a development identity + a wildcard provisioning profile that lists the phone | a real iPhone | Xcode → Settings → Accounts |
| Node.js ≥ 20 and Claude Code, logged in | the runner | `curl -fsSL https://claude.ai/install.sh \| bash` |
| Figma MCP server (optional) | design checks in runs | https://mcp.figma.com/mcp |

## Drive a device

```sh
git clone https://github.com/christi10/qapilot && cd qapilot
bin/qapilot screen                                   # every visible element: e12 * id | text | desc [bounds]
bin/qapilot do "tap id:login-username" "type $TEST_USER" "hidekb" "tap Next" "wait Password"
bin/qapilot shot home.png                            # .jpg = 540 px wide
bin/qapilot audit
bin/qapilot --device 00008110-0012345678ABCDEF up    # an iPhone: builds and starts the agent (first time ~2 min)
```

`--device <adb serial or iOS UDID>`, else `$DEVICE`, else `emulator-5554`.

As an MCP server for any agent — tools `screen`, `do`, `screenshot`, `audit`, `snapshot`, `release`, `design`:

```sh
claude mcp add qapilot -- /path/to/qapilot/bin/qapilot mcp
```

### Steps (`do`, one call per intent; it stops at the first failure and returns the final screen)

`tap <sel>` · `tap x,y` / `tap 66%,20%` · `type <text>` / `type $VAR` · `digits <d>` (taps an on-screen keypad if there
is one — PIN and OTP boxes drop fast input) · `erase` · `back` (iOS: edge swipe) · `enter` · `home` · `hidekb` ·
`swipe up|down|left|right` · `swipe x1,y1,x2,y2` · `scroll <sel>` · `wait <sel>` · `gone <sel>` · `sleep <ms>` ·
`launch <package/bundle>` · `stop <package>` · `otpmark` / `otp <user> [service]` · `shot <file> [<sel>]` ·
`name <screen>` · `goto <screen>` · `issue <KEY>` (the steps after it are that issue's replay).
`tap`, `swipe`, `launch` and `goto` wait until the screen stops moving and spinners are gone. Timeouts as `wait:30 …`.

Selectors: `e12` (from the last screen) · `id:` · `text:` · `desc:` · `re:` · plain words (contains, case- and
whitespace-insensitive). `bin/qapilot do --file flow.steps` replays a saved flow, one step per line.

Other commands: `snap list|save|load <name>` (emulator snapshots — a logged-in user back in seconds), `up` / `release`
(the on-device driver), `bench`, `design --figma <frame.png> [--figma-meta <get_metadata.xml>] [--out <dir>]`,
`cross --android <check dir> --ios <check dir>`.

### OTPs

`otp` reads SMS codes from a command you provide: `QAPILOT_OTP_CMD` prints the newest codes as a JSON array —

```json
[{ "id": "8812", "code": "123456", "user": "user1", "phone": "+15550100123", "service": "/login", "time": "2026-11-17T10:00:00Z" }]
```

(only `id` and `code` are required). `otpmark` before the tap that sends the SMS, then `otp user1 login` waits for a
code that was not there at the mark, for that user (or phone) and service, and types it.

## Let an AI agent test the app

Describe the app once in a profile ([examples/app.json](examples/app.json)):

```json
{
  "id": "shop",
  "name": "Shop",
  "package": "com.example.shop",
  "ios": { "bundleId": "com.example.shop" },
  "figma": [{ "name": "App design", "url": "https://www.figma.com/design/<fileKey>/Shop" }],
  "testData": { "user1": "two saved cards", "user2": "a new customer, nothing saved" },
  "login": "At the PIN screen: digits $TEST_PIN. At the login screen: type $TEST_USER and $TEST_PASSWORD.",
  "notesFile": "shop-notes.md",
  "sameWords": ["shop"],
  "onboarding": "flows/onboarding.yaml",
  "install": ["./install-latest.sh"]
}
```

Then:

```sh
export TEST_PASSWORD=… TEST_PIN=…                      # or KEY=value lines in ~/.qapilot/.env
bin/qapilot-run run --app examples/app.json --user user1 --issue examples/issues/PRJ-1.md
bin/qapilot-run run --app examples/app.json --user user1 --flow "Add a product to the cart and pay with a saved card"
bin/qapilot-run run --app examples/app.json --user user1 --explore "Checkout first"   # the whole app
bin/qapilot-run run … --device 00008110-0012345678ABCDEF                              # the same on an iPhone
bin/qapilot-run resume <run> --answer "The SMS code is 123456"                        # a run that asked for something
bin/qapilot-run combine <android run> <ios run>                                       # one report, both platforms
```

A run detaches and prints its folder (`~/.qapilot/runs/<label>-<time>/`, or `--runs <dir>`); `--fg` keeps it in the
terminal. It leaves:

- `report.md` — results, the design check per screen (Figma and app side by side, the differences with both values),
  and every other bug written for the developer: component, steps, expected / actual with numbers, a suggested fix.
- `result.json` — a verdict per issue (`pass | fail | blocked | not-testable`, with confidence and evidence), the
  findings, and for a whole-app run the coverage (every screen visited, and what could not be reached and why).
- `evidence/`, `design/`, and `flows/<KEY>.steps` — each issue's steps as they replay (`bin/qapilot do --file …`).

**Issues** are Markdown files: `# PRJ-1: What is wrong` first, then anything a tester would read — steps, expected
and actual results, screenshots next to the file. `--issues <dir>` takes every `*.md` (or `<KEY>/issue.md`) in it.
The agent never writes to your tracker; export the issues, run, and file the findings yourself.

**Profile fields**: `package` (Android) and/or `ios.bundleId`; `figma` links (design checks; the frames are cached
in `~/.qapilot/figma/cache`); `testData` (what each test user has — a run switches users through emulator snapshots
`<app>-<user>` when an issue needs data the first one lacks); `login` (how to get past the app's login);
`notesFile` (what the agent must know about the app: element ids, known bugs, traps); `sameWords` (brand words the
language check must not flag); `onboarding` (a Maestro flow that logs a fresh user in — `--onboard`, then saved as a
snapshot); `install` (a command that installs the latest build — `--install`, run with `APP_ID`, `CHANNEL`, `DEVICE`).

**Environment**: `TEST_PASSWORD`, `TEST_PIN` (the test user), `TEST_PIN_<USER>` (the others), `QAPILOT_OTP_CMD`,
`DEVICE`, `QAPILOT_AVD` (the emulator to boot when it is not running), `QAPILOT_HOME` (data, default `~/.qapilot`).
Secrets stay in the environment; the agent is told never to print them.

### From your own tools

The runner is a library too — a dashboard or a tracker integration can fetch the issues itself and start runs:

```js
import { run, readIssue, normalizeResult, combine } from './qapilot/runner/index.mjs';
const result = await run({ dir: 'runs/PRJ-1-…', app, appDir, user: 'user1', source: { issues: [readIssue('PRJ-1.md')] }, env });
```

`run()` takes hooks for whatever is special about your setup: `installCmd`, `onboardCmd`, `notes`, `login`, and
`claude` (`model`, `timeoutMinutes`, `allowBash` for your own scripts, `cwd`). See [runner/run.mjs](runner/run.mjs).

## Data

`$QAPILOT_HOME` (default `~/.qapilot`): `nav/<package>.json` (learned screen paths), `figma/` (screen → frame map,
frame index, cached frames), `snapshots.json`, `install-check.json`, `ios-agent/<udid>/` (agent builds), `runs/`.

## Development

```sh
cargo build --release && cargo test --release && cargo clippy --release
node --test 'test/*.test.mjs'
```

`prompts/playbook.md` is how a run tests; `templates/report.md` is the report it fills.

## License

MIT
