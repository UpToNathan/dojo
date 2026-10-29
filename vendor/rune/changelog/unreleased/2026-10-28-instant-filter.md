- **Filtering a `Combobox` or `PopupList` is instant by default.** A keystroke's rows and the
  popover's height used to crossfade and spring (0.3 s in, 0.24 s out); they now land in the
  frame, and a `PopupList` lands any change to its `items` at once, even in a commit that
  animates the field it follows (`animate_edit_layout`). Opening, closing and the hover
  highlight keep their motion. `.animate_filtering(true)` on either brings the crossfade back.
  New: `El::instant_changes()`, which lands every change inside an element (layout, opacity,
  transforms, presence of what it holds, growth rows) in any commit while its own entrance,
  exit and colours keep theirs. A scroll-into-view that mounts new virtual rows now commits
  them as instantly as the change that asked for it.
