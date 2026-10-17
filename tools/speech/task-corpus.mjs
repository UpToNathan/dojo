import { desktopReplay } from './desktop-extension.mjs';

const TIERS = new Set(['native_semantic', 'shell', 'browser', 'vision_only']);
const STATUSES = new Set(['satisfied', 'unsatisfied', 'unknown']);
const isAction = route => desktopReplay(route) === 'unsafe';

export function validateCorpus(corpus) {
  const errors = [];
  if (corpus?.version !== 1 || !Array.isArray(corpus.tasks) || corpus.tasks.length === 0) return ['Corpus needs version 1 and tasks'];
  const seen = new Set();
  for (const task of corpus.tasks) {
    const id = String(task?.id ?? '');
    if (!/^[a-z0-9][a-z0-9-]*$/.test(id)) errors.push(`Invalid task id: ${id}`);
    if (seen.has(id)) errors.push(`Duplicate task id: ${id}`);
    seen.add(id);
    if (!TIERS.has(task?.tier)) errors.push(`${id}: unknown tier ${task?.tier}`);
    if (typeof task?.goal !== 'string' || task.goal.length === 0) errors.push(`${id}: missing goal`);
    if (typeof task?.fixture !== 'string') errors.push(`${id}: missing fixture`);
    if (!Number.isSafeInteger(task?.max_actions) || task.max_actions < 1 || task.max_actions > 50) errors.push(`${id}: invalid max_actions`);
    if (!Array.isArray(task?.success) || task.success.length < 1 || task.success.length > 4) errors.push(`${id}: success needs 1 to 4 checks`);
    for (const check of task?.success ?? []) {
      if (typeof check?.fixture_log_contains === 'string') continue;
      if (!Array.isArray(check?.expect) || check.expect.length < 1 || check.expect.length > 8) errors.push(`${id}: a check needs 1 to 8 predicates`);
      if (check?.window !== null && typeof check?.window !== 'string') errors.push(`${id}: window must be a hint string or null`);
      for (const predicate of check?.expect ?? []) {
        if (predicate?.window === undefined && predicate?.element === undefined) errors.push(`${id}: predicate needs window or element`);
        if (/"(pid|window_id)"/.test(JSON.stringify(predicate))) errors.push(`${id}: predicates must not pin pid or window_id`);
      }
    }
  }
  return errors;
}

// Metrics from one exported run. Outcome comes from the runner's oracle; the
// agent's own desktop_verify use is reported separately, never as success.
export function runMetrics(task, events) {
  const actions = events.filter(event => event.kind === 'intent' && isAction(event.route));
  const receipts = events.filter(event => event.kind === 'receipt' && isAction(event.route));
  const verifications = events.filter(event => event.kind === 'verification');
  return {
    actions: actions.length,
    refused: receipts.filter(event => event.data?.result?.effect === 'refused'
      || (typeof event.data?.result?.reason === 'string' && event.data.result.handled === false)).length,
    errors: receipts.filter(event => event.data?.is_error === true).length,
    agent_verified: verifications.length > 0,
    agent_last_verification: verifications.at(-1)?.data?.status ?? null,
    within_budget: actions.length <= task.max_actions,
  };
}

// Statuses are the oracle's desktop_verify results, one per success check.
export function combineOracle(statuses) {
  if (statuses.length === 0 || statuses.some(status => !STATUSES.has(status))) return 'unknown';
  if (statuses.includes('unsatisfied')) return 'failed';
  if (statuses.every(status => status === 'satisfied')) return 'passed';
  return 'unknown';
}

export function scoreRun(task, events, oracleStatuses) {
  const metrics = runMetrics(task, events);
  let outcome = combineOracle(oracleStatuses);
  if (outcome === 'passed' && !metrics.within_budget) outcome = 'over_budget';
  return { task_id: task.id, tier: task.tier, outcome, oracle: oracleStatuses, ...metrics };
}

export const skipped = (task, reason) => ({ task_id: task.id, tier: task.tier, outcome: 'skipped', reason });
// The model never produced a usable run (provider error, usage limit): not evidence about the agent.
export const agentError = (task, error) => ({ task_id: task.id, tier: task.tier, outcome: 'agent_error', error: String(error) });

export function summarize(results) {
  const ran = results.filter(result => !['skipped', 'agent_error'].includes(result.outcome));
  const count = outcome => ran.filter(result => result.outcome === outcome).length;
  const byTier = {};
  for (const result of ran) {
    const tier = byTier[result.tier] ??= { runs: 0, passed: 0 };
    tier.runs += 1;
    if (result.outcome === 'passed') tier.passed += 1;
  }
  return { total: results.length, skipped: results.filter(result => result.outcome === 'skipped').length,
    agent_errors: results.filter(result => result.outcome === 'agent_error').length, ran: ran.length,
    passed: count('passed'), failed: count('failed'), over_budget: count('over_budget'), unknown: count('unknown'),
    pass_rate: ran.length === 0 ? null : count('passed') / ran.length,
    agent_never_verified: ran.filter(result => !result.agent_verified).length,
    refusals: ran.reduce((sum, result) => sum + result.refused, 0),
    tool_errors: ran.reduce((sum, result) => sum + result.errors, 0), by_tier: byTier };
}
