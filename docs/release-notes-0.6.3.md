# klipa 0.6.3

A memory fix. klipa's footprint no longer scales with the size of the
images in your history.

## The problem

Every image entry in the dropdown needs a small preview. Building one
decoded the whole PNG into memory first, then threw almost all of it
away. A retina screenshot is large once decoded: 3644x2372 pixels is
about 34 MB of raw RGBA, and the dropdown can show up to twenty entries.

The decoded buffers were freed immediately, but the allocator keeps those
pages, so the process stayed at its worst-case size for the rest of the
session. On a history full of screenshots that meant hundreds of
megabytes resident for a menu bar app, with the peak growing alongside
the resolution of whatever had been copied.

## The fix

- Previews are now built by reading the PNG one row at a time straight
  off disk and sampling each row into the thumbnail as it passes. Peak
  cost is a single source row plus the small output canvas, whatever the
  image's dimensions.
- The preview cache is bounded to the number of entries the dropdown can
  actually show, instead of growing with the history behind it.
- Pasting an image still has to materialise it in full for the system
  clipboard, so klipa now returns those pages to the OS afterwards rather
  than holding them for the rest of the session.

Measured over a real 69-image history, peak resident memory for building
every preview falls from 121 MB to 9 MB.

## Compatibility

No change to the history format, stored images, settings or licensing,
and nothing to migrate. Previews are byte-identical to the ones the
previous version produced, which a regression test now asserts.
Interlaced PNGs, which klipa never writes itself, fall back to the
previous whole-image path.

Keep awake, the menu-bar clock, search and the Windows and Linux builds
are unchanged.
