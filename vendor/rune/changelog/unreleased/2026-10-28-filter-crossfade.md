- **Filtering a virtual list crossfades what shows, and motion nobody sees ends at once.** A
  removed element that drew nothing last frame used to skip its exit only when it had already
  faded out. It now also skips it when it, and everything it holds, lay out of the window or
  clipped away, so filtering a long list no longer runs dozens of fades under its viewport's
  clip. In a virtual list, rows a change to the rows (a filter, a sort) mounts now enter with
  their insertion transition, and those that land out of view finish it as soon as they draw.
  A change that keeps none of the rows in view no longer anchors the scroll offset to a kept
  row further down, and the rows it removes no longer hold their space: they fade where they
  stood while the matches fade in at the top. A kept row that was out of view and lands in
  view enters too, rather than appearing whole. Rows scrolled into view still mount as they
  are. New: `Scene::enter_now` and `Scene::reenter`.
