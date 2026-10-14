# Evidence for a decision model in Dojo

Research checked on 2026-10-11. This note examines replacing specific decisions currently made through language generation with typed classification or scoring. It does not recommend choosing between general-purpose models for whole requests. No implementation or paid model experiment was performed.

## What the evidence supports

A decision-only model can make a bounded choice, answer a yes/no question, or score supplied material without generating an answer or explanation. It can replace an existing expensive decision call. Adding a decision call while keeping all existing work adds cost. The strongest evidence supports narrow, repeated judgments with observable ground truth. Evidence for unrestricted desktop planning is weaker.

TypeSafe's current API exposes typed questions over structured text state. Jev 1.13 is hosted, text-only, and charges $0.042 per million input tokens, with free outputs. Those are current published prices, not a guarantee of future pricing or successful decisions. Its context permits 64K tokens overall and 32K for state plus the longest question. The vendor advises pinning a version when using calibrated thresholds. [Official API](https://docs.typesafe.ai/api), [official model documentation](https://docs.typesafe.ai/models).

## Direct measurements

### Decision-only judging can be very cheap on suitable tasks

The October 6 JEV-as-a-Judge preprint compares seventeen judge configurations. Jev and GPT-6 Astra both reach 92.5% on 1,500 RewardBench pairs. Jev reaches 87.3% versus 88.4% on 3,000 evidence-grounded HaluEval judgments, but 78.6% versus 93.1% on 350 JudgeBench pairs. On a separate 120-item matched timing panel, estimated fees per 1,000 judgments are $0.044 versus $12.182, and median outcome latency is 0.15 versus 1.89 seconds. These fees use collection-time API usage and prices, not invoices. All judges return verdicts and probabilities without rationale, so the advantage survives a short-output comparison.

The paper's strongest combined quality-and-cost result uses an escalation cascade. That falls outside the user's requested scope. Human adjudication favors GPT-6 more than supplied labels do. Standalone results support replacing suitable judgments, with gaps on math, code, and logic. This recent preprint does not establish completed-task accuracy in Dojo. [Paper, Table 1 and sections 4–6](https://arxiv.org/html/2609.26550v4).

### General text decisions can lose substantial accuracy

A separate text-annotation preprint evaluates Jev across 15 evaluation tasks. It trails the best LLM selected for each task on 14 tasks, with a median deficit of 11.6 macro-F1 points and median measured cost 44 times lower. The best-per-task comparator is selected on test results, which favors the LLM comparison. Nonetheless, cheap decisions have meaningful accuracy costs. Typed responses eliminate parsing errors across 23,703 registered decision-model calls; that does not establish semantic correctness. The study also finds that high confidence can fail on some tasks, including empathy. This directly rejects the claim that a probability output automatically makes a decision trustworthy. [Paper, sections 3–6](https://arxiv.org/html/2609.24574v2).

### Replacing an interpreter can reduce fees per completed job

A two-node OCR service study holds the validator and scheduler fixed and substitutes Jev for a concise structured-output DeepSeek interpreter. Across eight paired conditions, Jev matches correctly completed, on-time job counts in seven and exceeds them in one. API fees per correct completion are 69.0–70.6% lower. This directly measures completed service output rather than label accuracy alone. It interprets four bounded intent fields and is not a general computer-use benchmark. Fees exclude execution compute, networking, energy, maintenance, and warmups; they are not total ownership cost. Repeated-request caching largely removes the latency advantage. This is also a recent preprint. [Paper, sections IV–V](https://arxiv.org/html/2609.22753v1).

### Tool selection has promising evidence, but savings must include overhead

Fast Models, Slow Evidence evaluates Jev and Laya on 11 agent decision points using 7,283 base cases and 6,640 robustness variants. Jev reaches about 98% tool selection with 50 similar tool candidates, versus 31% for Laya, on cases with a unique correct tool. These are bounded selection tests, not end-to-end execution with correct arguments and effects.

The paper audits its own deployment claims. A safety pre-screen initially reported 23.9% savings; including the decision call's tokens reduces that to 4.3%. A more conservative threshold costs 4.7% more than the reviewer alone. Thresholds selected and evaluated on the same data also hide extra misses. Its real-context relevance checks support caution about skipping answers based on a gate. This preprint is useful as evidence for a possible mechanism and as a warning about incorrect cost accounting. [Paper, sections 5–7](https://arxiv.org/html/2610.02267v1).

### Published specialist-model evidence is stronger than a universal decision claim

MiniCheck, published at EMNLP 2024, trains a 770M-parameter grounded fact checker on synthetic examples. It approaches GPT-4 accuracy on the 13K-item LLM-AGGREFACT test set. Its reported inference cost is $0.24 versus $107 for GPT-4. The small-model figure converts runtime on the authors' hardware into cloud GPU cost at $0.80 per GPU-hour; GPT-4 uses API prices. The comparison excludes building a new full agent and is not a laptop energy measurement. It proves that a narrow verification decision can become much cheaper, with training and the right evidence format. It does not prove a generic decision model can verify arbitrary desktop actions or software correctness. [Paper, Tables 2–4 and section 6.2](https://aclanthology.org/2024.emnlp-main.499.pdf).

### Cached candidate scoring can miss the governing rule

Can a Cacheable Decision Model Follow Rules compares a joint input-candidate scorer with cheaper separately encoded candidates. On its rule-sensitive 17-candidate slice, joint scoring reaches 1.00 accuracy, while two cacheable scorers fall to 0.24 and 0.08. A shortlist needs all 17 candidates to recover complete inclusion in the measured sizes. These are synthetic, constrained experiments. The relevant warning is that similarity scoring can select a plausible action while ignoring the rule that makes another action correct. Cheap embeddings alone are not sufficient evidence of policy compliance. [Paper, sections 3–5](https://arxiv.org/html/2609.37832v1).

## Where Dojo could benefit

### Browser automation and computer use

Jev-Mobile directly tests GUI execution on the full 116-task AndroidWorld suite. It reports 79% task success versus 84% for a step-wise vision-language model. Among successful trajectories only, mean model API cost is 73.4% lower and mean execution time 32.7% lower. This is a recent preprint comparing complete systems, without an isolated Jev ablation. The successful-only averages do not measure total cost per successful request including failed attempts.

Its mechanism is concrete. A vision-language model occasionally supplies a local goal and exact input strings. Code enumerates executable candidates from the current accessibility tree. Jev receives text, selects a candidate ID or DONE/BLOCKED, and can execute several steps before another planning call. Code validates the selection against a fresh observation and rebuilds candidates after every action. Jev is hosted; local execution scope does not mean on-device inference. Missing accessibility targets fail because this version has no visual coordinate fallback. [Paper, sections 3, 5–8](https://arxiv.org/html/2609.30186v1).

Applied to a browser, the same mechanism would use DOM or accessibility labels and actual action handles. For example, a planner supplies the goal and search text; a bounded executor selects the visible search field, submits the supplied text, selects a matching result, and re-observes after each action. This is a proposed adaptation, not a browser result from the Android paper. Screenshot-only computer use needs a separate model to identify actionable controls, because a text-only decision model cannot see pixels. A missing or stale candidate should return control rather than invent a click location. Explicit postconditions must verify the task; a model's DONE is only a handoff signal. Dojo's current desktop observation server explicitly says it does not supply accessibility observations, so this structured desktop candidate source would need new work. [Observation boundary](../../src/bin/dojo-agent.rs).

The current speech spec sends requests to T3 using its configured default model and existing subscription credentials. The coordinator chooses that selection and provides the assistant instructions. There is no demonstrated separate, per-call expensive judge to replace today. [Speech spec](../speech/spec.md), [coordinator](../../tools/speech/voice.mjs), [assistant instructions](../../tools/speech/assistant.md).

Potential bounded uses, inferred from the research rather than validated for Dojo, include selecting a small set of relevant tools from a fixed catalogue, matching an utterance to a known desktop intent, checking whether supplied evidence supports a claim, and choosing among already valid candidate actions. Deterministic program checks should own facts that are already available structurally, such as whether a process started, a requested setting changed, or a command exited successfully.

A selector that only passes its opinion into the existing T3 run may save no calls at all. A model that can choose an intent may still need a generator for arguments, dialogue, or recovery. Current subscription authentication also means lower token consumption does not automatically lower the monthly bill. Adding a separately billed hosted decision service can increase dollars spent even while lowering generative work.

For a Dojo evaluation, count every decision, generation, retry, and recovery call. Measure cost per successfully completed request alongside false actions, argument errors, latency, and explicit abstentions. Use held-out real requests, including ambiguous speech and repeated near-identical tools. Compare the current agent with both a deterministic baseline and a bounded decision replacement. Keep the same final success checks. Only claim savings when actual work disappears and completed-task quality remains acceptable.

The evidence is sufficient to justify a narrow experiment. It is not sufficient to promise a percentage saving for Dojo or to add a general decision layer in front of every utterance.
