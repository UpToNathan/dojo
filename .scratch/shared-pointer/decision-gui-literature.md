# Decision models and screenshot search

Checked October 11, 2026. This is a literature review, with no Dojo performance measurements or model calls.

## Evidence that matches regional screenshot search

[RankGround, September 2026](https://arxiv.org/html/2609.18690v1) provides the closest match. It trains a 2B multimodal reranker to score overlapping screenshot crops. A separate GUI model predicts a click from the best crop. Table 2 reports these ScreenSpot-Pro results with Qwen3-VL-8B:

| Method | Click accuracy | Latency |
| --- | --- | --- |
| Full screenshot | 48.72% | 814 ms |
| Two-call zoom | 62.00% | 1,486 ms |
| Ranked crop | 64.26% | 959 ms |

The classifier therefore beats repeated grounding calls for speed and accuracy, but adds latency over one full-screen call. Its reranker costs approximately 145 ms. The paper measures preprocessing and postprocessing, excludes data loading, and does not measure hosted billing. These results require GUI-specific training. The crop ranker's Hit@1 is 88.2%; selecting the wrong crop prevents the coordinate model from recovering. Repeated icons remain difficult. ScreenSpot-V2 also has a slight accuracy regression against direct inference. This is evidence for selective use, rather than cropping every observation.

## Evidence for refinement and verification

[GUI-Lens, August 2026](https://arxiv.org/html/2608.03270v1) combines OCR/component references, model-selected crops, and visual verification. It can reject a bad crop and return to a broader view. Its efficient configuration improves accuracy by 10.34 percentage points at 1.27 times single-shot latency on a 300-example ScreenSpot-Pro subset. Token usage was unavailable, so this result cannot establish money savings. The paper also reports interactive OSWorld results, beyond static click localization. It uses general VLMs, rather than Jev, Clef, or Decisions.

[ShowUI, CVPR 2025](https://openaccess.thecvf.com/content/CVPR2025/papers/Lin_ShowUI_One_Vision-Language-Action_Model_for_GUI_Visual_Agent_CVPR_2025_paper.pdf) reports 75.1% ScreenSpot grounding accuracy with a 2B model. Its visual-token selection removes 33% of redundant visual tokens and speeds training by 1.4 times. That is a training result, not proof of equivalent request latency or savings from an external decision API.

## What is established for the proposed providers

The reviewed [Clef evaluation](https://blog.cloudflare.com/clef-decision-models/), [Jev model reference](https://docs.typesafe.ai/models), and [Decisions guide](https://developers.openai.com/api/docs/guides/decisions) do not establish GUI crop-search accuracy or end-to-end desktop task savings. Clef and Decisions accept images. Jev needs textual input from OCR, accessibility data, or another vision model. Applying their generic decision interfaces to crop selection is a proposed experiment.

## Candidate Dojo uses

These are design inferences from the literature and provider capabilities:

- Rank application windows or overlapping crops for a known target. Ask the main agent to inspect only the best candidate, with alternatives or a full-screen fallback when uncertain.
- Check observable outcomes after an action, such as a sidebar opening or a dialog appearing.
- Classify a fresh observation as ready, loading, blocked, or requiring a new plan.
- Route a step to an application API, accessibility data, or visual grounding.
- Decide whether a changed screen warrants waking the larger planner. Use compositor events and ordinary image comparison before a model when those signals suffice.

Prefer actual window and panel bounds over a blind grid. Preserve a small full-screen overview so identical icons retain application context. Keep crop offsets, scale, and observation revision outside the model; code maps coordinates and rejects stale actions. A confident answer never overrides human ownership.

Savings depend on the model avoiding larger calls or retries. Sending every crop independently, or following every decision with an unchanged full-screen planner call, can increase cost. Measure total task success, wrong-crop rate, retries, latency, and all model charges before choosing a provider.
