# Agent pointer

`agent-pointer.png` is the actual `nf-fa-mouse_pointer` glyph, U+F245, exported
from Dojo's bundled `VictorMonoNerdFontMono-Regular.ttf`. Its cmap names the glyph
`fa-mouse_pointer`. The glyph is cropped to its ink bounds, colored cyan and
stored at 4x resolution. Niri uses buffer scale 4, giving a 24 logical pixel high
cursor. The top-left tip is the pointer hotspot. There is no hand-drawn arrow.

The PNG is embedded in Niri, so installed fonts and Fontconfig do not affect the
cursor. Its buffer exists only while the agent seat exists. Fractional outputs
resample the high resolution glyph through Smithay's existing cursor renderer.

The source font's SIL Open Font License is retained in
`agent-pointer-LICENSE.txt`. The Font Awesome glyph is also licensed under the
[SIL Open Font License](https://fontawesome.com/v4/license/).

From the Dojo repository, regenerate with a Python environment containing Pillow
and fonttools:

```sh
python3 scripts/export-agent-cursor.py
```

The packaged PNG and license allow this Niri subtree to build independently.
The committed asset was generated with Pillow 12.3.0 and fonttools 4.66.1.
