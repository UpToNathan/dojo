import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { validateCorpus, runMetrics, combineOracle, scoreRun, skipped, agentError, summarize } from './task-corpus.mjs';

const corpus = JSON.parse(readFileSync(new URL('./task-corpus.json', import.meta.url)));
const task = corpus.tasks[0];
const intent = route => ({ kind: 'intent', route });
const receipt = (route, result, isError = false) => ({ kind: 'receipt', route, effect: 'unknown', data: { result, is_error: isError } });
const act = [intent('desktop_capture_action'), receipt('desktop_capture_action', { handled: true })];

test('the shipped corpus is valid and covers every tier', () => {
  assert.deepEqual(validateCorpus(corpus), []);
  assert.deepEqual([...new Set(corpus.tasks.map(t => t.tier))].sort(), ['browser', 'native_semantic', 'shell', 'vision_only']);
});

test('validation rejects pinned windows, duplicates, bad budgets and bad hints', () => {
  const pinned = { ...task, id: 'pinned', success: [{ window: 'Alpha', expect: [{ window: { exists: true }, window_id: 3 }] }] };
  const errors = validateCorpus({ version: 1, tasks: [task, task, pinned, { ...task, id: 'budget', max_actions: 0 },
    { ...task, id: 'hint', success: [{ window: 7, expect: [{ window: { exists: true } }] }] }] });
  for (const needle of ['Duplicate', 'pin pid or window_id', 'max_actions', 'hint string or null']) assert(errors.some(e => e.includes(needle)), needle);
});

test('the oracle decides, not delivery or the agent’s own verification', () => {
  const claimed = [...act, { kind: 'verification', route: 'desktop_verify', data: { status: 'satisfied' } }];
  assert.equal(scoreRun(task, claimed, ['unsatisfied', 'satisfied']).outcome, 'failed');
  assert.equal(scoreRun(task, act, ['satisfied', 'satisfied']).outcome, 'passed');
  assert.equal(scoreRun(task, act, ['satisfied', 'unknown']).outcome, 'unknown');
  assert.equal(scoreRun(task, act, []).outcome, 'unknown');
  assert.equal(scoreRun(task, act, ['satisfied', 'bogus']).outcome, 'unknown');
  assert.equal(scoreRun(task, act, ['satisfied']).agent_verified, false);
  assert.equal(scoreRun(task, claimed, ['satisfied']).agent_last_verification, 'satisfied');
});

test('the action budget turns a pass into over_budget', () => {
  const many = Array.from({ length: task.max_actions + 1 }, () => act).flat();
  const result = scoreRun(task, many, ['satisfied']);
  assert.equal(result.outcome, 'over_budget');
  assert.equal(result.within_budget, false);
});

test('refusals and tool errors come from the real window-changed receipt shape', () => {
  const refusal = { handled: false, effect: 'unknown', reason: 'capture window identity or rendered geometry changed', requires_fresh_observation: true };
  const metrics = runMetrics(task, [intent('desktop_capture_action'), receipt('desktop_capture_action', refusal),
    intent('desktop_capture_action'), receipt('desktop_capture_action', { text: '[redacted]' }, true), intent('desktop_observe')]);
  assert.deepEqual({ refused: metrics.refused, errors: metrics.errors, actions: metrics.actions }, { refused: 1, errors: 1, actions: 2 });
  const semantic = runMetrics(task, [intent('desktop_accessibility_action'),
    receipt('desktop_accessibility_action', { effect: 'refused', reason: 'No such file or directory (os error 2)', reference_consumed: true })]);
  assert.equal(semantic.refused, 1);
});

test('combining oracle statuses never lets unknown hide a failure', () => {
  assert.equal(combineOracle(['unknown', 'unsatisfied']), 'failed');
  assert.equal(combineOracle(['satisfied']), 'passed');
});

test('summaries exclude skipped tasks and report agent verification and refusals', () => {
  const ok = scoreRun(task, [...act, { kind: 'verification', route: 'desktop_verify', data: { status: 'satisfied' } }], ['satisfied']);
  const quiet = scoreRun(corpus.tasks[1], act, ['unsatisfied']);
  const summary = summarize([ok, quiet, skipped(corpus.tasks[5], 'no adapter')]);
  assert.deepEqual({ total: summary.total, skipped: summary.skipped, ran: summary.ran, passed: summary.passed, failed: summary.failed },
    { total: 3, skipped: 1, ran: 2, passed: 1, failed: 1 });
  assert.equal(summary.pass_rate, 0.5);
  assert.equal(summary.agent_never_verified, 1);
  assert.deepEqual(summary.by_tier.native_semantic, { runs: 2, passed: 1 });
  assert.equal(summarize([skipped(task, 'x')]).pass_rate, null);
});

test('provider failures are not agent evidence and stay out of the pass rate', () => {
  const failed = agentError(task, 'Codex error: The usage limit has been reached');
  assert.equal(failed.outcome, 'agent_error');
  const summary = summarize([failed, scoreRun(task, act, ['satisfied'])]);
  assert.deepEqual({ ran: summary.ran, agent_errors: summary.agent_errors, pass_rate: summary.pass_rate }, { ran: 1, agent_errors: 1, pass_rate: 1 });
  assert.equal(summarize([failed]).pass_rate, null);
});
