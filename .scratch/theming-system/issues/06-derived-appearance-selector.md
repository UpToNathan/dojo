# Apply the selected derived appearance

Status: resolved

Presets already switch darkman to their authored appearance. Derived themes must apply the selected Dark or Light appearance too, instead of using it only for preview. Selection remains a preview until Apply. Generate and validate outputs before switching system appearance, matching preset behavior.

Derived Apply now captures the selected appearance, switches darkman after generation validation, and applies that appearance. The selected button is highlighted. Fixed shell reload coalescing so a queued file event cannot discard an already-cached color update. Picker and configuration tests pass; installed in the running shell.
