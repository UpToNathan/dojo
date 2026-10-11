# Would Clef save Dojo money?

Checked October 10, 2026. This is pricing research and illustrative arithmetic, not measured savings. No integration or paid requests were made.

## Conclusion

Clef-flash can reduce metered model costs when it replaces screenshot interpretation or intermediate observation turns. Adding a validator while T3 continues to receive and interpret every screenshot adds cost. For a user paying a fixed ChatGPT subscription and staying within its included allowance, fewer agent observations do not reduce the subscription bill. They may preserve usage capacity or avoid buying extra credits.

OpenAI explicitly separates API prices from included subscription usage and says not to use API token prices to estimate included tasks. Its documentation says context, tools, reasoning, and caching affect subscription usage. See [ChatGPT and Codex pricing](https://learn.chatgpt.com/docs/pricing).

Dojo currently has desktop context, action, and release tools, with no screenshot or accessibility observation service. The proposal needs a screenshot pipeline. Savings from avoided agent turns also require the tool to handle action, waiting, and verification before returning to T3. Returning after each click still invokes the main model for its next step.

## Published prices

Clef-flash costs $0.038 per million input tokens with no output charge. It accepts multiple typed questions in one request. Images count as input tokens: one token per 32-by-32 patch after resizing, plus three markers, capped at 1,024 tokens per image. Questions and state add tokens. See [Clef-flash documentation](https://developers.cloudflare.com/workers-ai/models/clef-flash/).

Workers AI includes 10,000 free Neurons daily, shared across models, resetting at 00:00 UTC. Clef-flash uses 3,455 Neurons per million input tokens. That gives approximately 2.9 million tokens, or 1,447 checks of 2,000 tokens each, per day if used exclusively for these checks. Excess usage needs the paid plan or an applicable billing route. See [Workers AI pricing](https://developers.cloudflare.com/workers-ai/platform/pricing/).

Standard short-context GPT-6.1 Sol API rates are $2 per million uncached input tokens, $0.10 cached input, $2.50 cache writes, and $10 output. These rates describe API use, not the current account's included allowance. See [OpenAI API pricing](https://developers.openai.com/api/docs/pricing).

## Illustrative arithmetic

Assume a Clef request consumes 2,000 total input tokens. Separately assume a main-model observation consumes 2,000 uncached input tokens and 200 output tokens. These are chosen workload sizes. Identical screenshots need not produce identical token counts across providers: OpenAI's image cost depends on the model and detail settings. See [OpenAI image billing](https://developers.openai.com/api/docs/guides/images-vision#calculating-costs).

| Observation method | Cost per check | Cost for 10,000 checks |
| --- | ---: | ---: |
| Clef-flash, before free allocation | $0.000076 | $0.76 |
| GPT-6.1 Sol under the assumptions above | $0.006 | $60.00 |

This is a 98.7% reduction for that isolated observation workload if Clef fully replaces it. It is not a 98.7% reduction in a desktop task's total bill. Planning, action selection, unavoidable agent turns, and escalations remain.

An avoided main-model turn can also avoid resending its existing context. For example, 50,000 cached Sol tokens add $0.005 per turn. Reusing context is cheaper than repeatedly treating it as uncached: cache hits depend on matching prefixes and API cache behavior. See [OpenAI prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching).

The accounting rule is:

`net savings = removed main-model cost - Clef cost - added summaries, retries, and escalations - any added fixed charges`

If no main-model work is removed, there are no direct savings. False positive outcomes can create expensive recovery work, so judge cost per successfully completed task rather than price per check.

## Economical implementation to evaluate

Keep checks inside an action-and-verify tool call. Return a compact outcome to T3 on success; return a relevant screenshot when the outcome is uncertain, unexpected, or failed. Only suppress observations that the planner does not need to choose its next action.

Ask related questions about the same screenshot in one request instead of uploading it separately for each question. Prefer a current cropped screenshot when sufficient. Before-and-after pairs double the image component and should be used when comparison is necessary. A crop must retain enough detail to judge the outcome correctly.

Check after meaningful actions or relevant application changes. Rate-limit repeated checks while waiting for a condition, stop on success or timeout, and avoid checking an unchanged screen continuously. Use compositor events directly for focus and human ownership because those facts do not require a model. Escalate ambiguous outcomes without automatically replaying clicks.

A useful evaluation compares the same desktop tasks with and without Clef and records actual input, cached input, output, verification requests, agent turns, completion success, wrong continuations, recovery actions, and latency. Report API savings separately from subscription allowance consumption.
