# Capture design decision

Use candidate A's compatible output plus optional region/window_id/scale request,
with a validated capture plan in the screenshot module. Graft candidate B's
post-capture geometry check and metadata derived from the actual PNG dimensions.
Reject mixed selectors at the boundary. Scale is limited to 0.1 through 1.

Correct candidate A's coordinate assumption: floating tile positions are relative
to the output's workspace view. Add the output's global origin only for grim -g.
The current compositor omits tiled positions, so do not infer them. Niri currently
exports a decimal foreign-toplevel identifier matching its mapped ID, contrary
to the sketches' namespace assumption; grim -T nevertheless fails here because
the running compositor lacks the required capture protocol. Use visible crops.
Niri's screenshot action always modifies the clipboard, so it is not used.

Cross-judge recommended candidate A with corrected coordinates and explicit
window limitations. Post-capture comparison is retained to reject changed bounds.
No generic capture framework, new dependency, or compositor modification is needed.
