# Long-response bubble animation

Type: task
Status: resolved

The transcript glitches intermittently when it rises above a long response.

## Answer

Captured a long-reply sequence with alternating sharp and blurred backgrounds
inside the transcript. Each bubble was committing a partial blur-region update
before the frame pixels. Both regions now update together, and Wayland applies
blur state with the rendered buffer instead of separate metadata-only commits.
Transcript lift now uses GPUI shaped text height rather than estimated wrapping.

Built and activated one resident shell; speech state tests passed. Captured
three long-response transitions before and after, with inspected samples showing
stable blur after the change. Evidence: /tmp/dojo-bubble-glitch/long-before and
/tmp/dojo-bubble-glitch/long-after. Reproduce with
scripts/capture-speech-animation.py --long --evidence /tmp/dojo-animation.
Visual captures require inspection; the script does not automatically classify
flicker. User confirmation of the intermittent symptom remains useful.
