- **Big shadows and border rings cost a fraction of what they did.** A window-sized panel's
  blurred shadow shaded every pixel under the panel each frame, running the erf shadow math
  where the panel hides it, and a border-only ring shaded its whole empty middle. A shadow
  under a solid box, and a ring with no fill, now draw as four bands around the middle they
  leave blank, and a shadow skips its erf rows well inside the box. Output matches to within one
  colour level; on a 5K window with a large panel the frame pass went from about 13 ms to about 4.5 ms.
