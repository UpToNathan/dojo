# Decision models for Dojo

Research checked on October 10, 2026. These findings cover provider documentation, not measured Dojo performance. No model requests or integrations were made.

## What the providers offer

### Jev from TypeSafe

The provider is **TypeSafe AI**, at `typesafe.ai`. Its docs identify the current version as `jev-1.13.0`, with `jev-latest` resolving to that version. Jev accepts text and structured JSON, with no image, audio, or video input. Its request limit is 64k tokens overall, including a 32k limit for the state plus the longest question. Price is $0.042 per million input tokens, with output free. The hosted endpoint is `POST https://api.typesafe.ai/v1/systemone`. These facts come from the [official model reference](https://docs.typesafe.ai/models).

Questions use `choice`, `score`, or `noul`. Choice returns an allowed value and option probabilities. Score returns a probability-weighted level index. Noul returns the probability of a yes answer. Multiple questions share one state. See the [official quick start](https://docs.typesafe.ai/introduction/quickstart).

TypeSafe reports 70–500 ms end-to-end responses in its [launch comparison](https://typesafe.ai/blog/introducing-system-one-models-and-jev). That is a provider claim tied to its workloads and network location. The reviewed official sources establish hosted access, but do not establish a downloadable Jev weights release or supported local deployment.

TypeSafe's `confidence` summarizes concentration in the option distribution. It is not a separate measurement of correctness. The provider also documents weaknesses in arithmetic, time comparisons, option order, irrelevant context, and adversarial content. See [confidence](https://docs.typesafe.ai/confidence) and [Jev 1.13 limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13).

### Clef and Clef-flash from Cloudflare

Both accept text, JSON, images, and video, and return probabilities over allowed answers. Workers AI lists [Clef](https://developers.cloudflare.com/workers-ai/models/clef/) as a 27B model with a 65,536-token context and $0.24 per million input tokens. It lists [Clef-flash](https://developers.cloudflare.com/workers-ai/models/clef-flash/) as a 9B model with a 24,576-token context and $0.038 per million input tokens. Neither charges for output. Media contributes input tokens.

Cloudflare publishes Apache 2.0 weights and code for [Clef](https://huggingface.co/Cloudflare/clef) and [Clef-flash](https://huggingface.co/Cloudflare/clef-flash). The model cards document a custom schema head and a Jev-compatible `/v1/systemone` path. Local execution must retain that decision head. Generic chat-generation examples shown by hosting sites do not establish equivalent decision behavior. The Clef-flash card documents a tested H200 setup. These sources do not establish acceptable latency or a supported quantized setup on this laptop.

Cloudflare's [launch evaluation](https://blog.cloudflare.com/clef-decision-models/) reports median request latencies of 209.3 ms for Clef and 38.8 ms for Clef-flash, with p95 values of 238.6 ms and 122.4 ms. Those are Cloudflare's benchmark results, not measurements against Dojo screenshots or this machine. Its quality tables show different winners across tasks, so the results do not justify declaring one model universally better.

### OpenAI Decisions API

The [official OpenAI guide](https://developers.openai.com/api/docs/guides/decisions) documents public beta access with `gpt-6-luna` as the only supported model, through `POST /v1/decisions`. Questions use `predicate`, `choice`, and `score`. Predicates return a probability. Choice and score answers return distributions and confidence. Input costs $0.10 per million tokens, with no output or cache charges. Regional and long-context premiums can apply. OpenAI claims roughly 10 times the speed of the Responses API, without giving a universal latency guarantee.

The [API reference](https://developers.openai.com/api/reference/typescript/resources/decisions/methods/create) accepts text and images, including up to 128 images per request. Audio, files, tools, and item references are unsupported. Questions can return refusals. The reviewed documentation provides hosted API access and does not establish local weights.

## Where a model fits in Dojo

The [coordination spec](spec.md) gives the compositor immediate responsibility for human takeover, held-input release, and stale-action rejection. Those operations remain deterministic. A model runs asynchronously above that layer and cannot override human ownership.

An agent runtime can ask a bounded question after a takeover, failed action, or changed observation. Candidate decisions are `continue`, `wait`, `reobserve`, `replan`, and `escalate`. Code limits the allowed actions using the current ownership snapshot before asking the model.

Each request carries its observation and coordination revision. A returned decision is discarded when its revision is stale. Every subsequent desktop action still passes the compositor's revision guard. A timeout, refusal, or uncertain answer produces a pause or fresh observation, rather than replaying a stale click.

Screenshot predicates are useful for observable outcomes such as whether a sidebar opened, whether a dialog appeared, or whether an expected control is visible. They supplement action verification. They do not prove that an application accepted an input event. Jev needs a separate text extraction or vision stage for those questions. Clef and Decisions can receive the screenshot directly.

## Initial recommendation

Evaluate **Clef-flash first** for screenshot outcome checks and bounded next-step decisions. Its published vision support, lower hosted price, and available weights make it the strongest starting candidate for Dojo's eventual local deployment. Compare it with **OpenAI Decisions** on the same labeled desktop cases. Include Jev when the input is already structured text, without adding a vision conversion stage solely to use it.

This recommendation is an inference from documented capabilities. A provider winner requires application evidence. Compare incorrect continuations after takeover, missed UI changes, unnecessary pauses, abstentions, and end-to-end latency. Calibrate thresholds per task and provider. A concentrated distribution does not grant permission to act, and typed output does not guarantee a correct judgment.

Run these checks only while an agent session needs them. Hosted requests naturally permit no persistent local model process. Local open weights add load time and memory costs that need measurement before adopting an idle shutdown policy.

## Free allowance and pricing comparison

Checked against [Workers AI pricing](https://developers.cloudflare.com/workers-ai/platform/pricing/) on October 10, 2026. Workers AI includes 10,000 free Neurons per day, shared across models. The allowance resets at 00:00 UTC. The Free plan stops requests when the limit is exhausted; going beyond it requires Workers Paid, which [starts at $5 per month](https://developers.cloudflare.com/changelog/post/2026-07-28-models-require-workers-paid/).

The published conversions are 3,455 Neurons per million Clef-flash input tokens and 21,818 per million Clef input tokens. Calculated from those rates, the daily allowance covers approximately 2.9 million Clef-flash tokens or 458,000 Clef tokens when used exclusively for that model. Media and question tokens contribute to usage. This is a daily allowance, not a bank of monthly credits.

Clef-flash's $0.038 per million input tokens is 62% below Decisions' $0.10. Full Clef's $0.24 is 2.4 times Decisions' rate. All three charge only for input. The [Decisions guide](https://developers.openai.com/api/docs/guides/decisions#pricing-and-availability) lists no recurring free allowance, and notes regional and long-context premiums.
