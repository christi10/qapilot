// The runner as a library, for tools that start runs themselves (a dashboard, an issue tracker integration).
export { ROOT, QAPILOT, home } from './util.mjs';
export { VERDICTS, FINDING_KINDS, SEVERITIES, CONFIDENCES, DESIGN_STATES, ID_RE, FIGMA_RE, DEVICE_RE, IOS_UDID_RE, safeRel, normalizeResult, replayFlows, saveFlows, sameBug, runPlatform } from './result.mjs';
export { MAX_ISSUES, runLabel, runId, readIssue, snapshotName, otherUsers, installCheckFresh, snapshotMatches, run, resume } from './run.mjs';
export { combine } from './combine.mjs';
