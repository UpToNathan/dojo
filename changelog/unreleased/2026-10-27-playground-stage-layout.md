- **A story picks how the playground's stage lays it out.** `Story::stage(Stage::Scroll)` and
  `Playground::stage(scene, Stage::Scroll)` make it a page for content taller than the stage:
  laid out at the stage's width inside a 24 pt margin, at its own height, from the top, and the
  stage scrolls. `Stage::Fill` puts it edge to edge across the whole stage, whatever size is
  picked beside it. `Stage::Center`, the default, lays out as before. `rune::Stage` is new at
  the crate root, so an app that glob-imports `rune::*` beside another `Stage` names one of them.
  The story list now shows each group's story count on its header, and a motion mark on stories
  and scenes with triggers.
