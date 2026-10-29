- **Trailing `RollingText` digits hold still in a width-leading row.** A trailing label's glyphs
  slid on the label's own spring to cancel their row's slide, so in a parent that slides on
  another spring (`width_leads`, say) they drifted 2–3 pt out and back as the label widened. The
  label and its glyphs now ride their parent's trailing edge: `El::ride_trailing()` (and
  `rune_motion::Scene::set_ride_trailing`) keeps an element against its parent's right edge while
  the parent's width animates, so glyphs that stay, leave or enter hold their columns whatever
  springs the rows around them use. The motion audit's odometer scenario adds header rows with a
  usage group, and `analyze.py` no longer splits a motion at a frame or two of rest, so an
  out-and-back through its start reads as a `reversal`.
