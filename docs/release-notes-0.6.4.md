# klipa 0.6.4

A second memory fix, in the other half of the image path. 0.6.3 stopped
the dropdown from decoding whole images to draw previews. This one stops
the watcher from decoding them at all.

## The problem

klipa polls the clipboard twice a second to notice what you copy. Text
takes a cheap path and returns immediately. An image does not: with no
text on the clipboard, every single poll asked the system for the image,
which materialises the whole picture as raw RGBA, and then hashed every
byte of that buffer to work out whether it had seen it before. The answer
was always no change, and the work was thrown away.

That ran twice a second for as long as the image stayed on the clipboard.
Copy a screenshot, leave it there, and klipa would sit at roughly a third
of a CPU core while its memory climbed without limit. Measured on a
3644x2372 screenshot, resident memory passed 2 GB in thirty seconds and
was still rising.

Nothing had to go wrong to trigger it. Taking a screenshot and not
pasting it anywhere was enough.

## The fix

macOS keeps a change counter on the pasteboard and bumps it on every
write. Reading it costs nothing and answers the only question a poll
actually has: is there anything new here. klipa now checks that first and
stops immediately when the answer is no, before touching any content.

Measured on the same image, sampled every five seconds:

| | CPU | Resident memory |
| --- | --- | --- |
| 0.6.3 | 30% | 387 MB, reaching 2.15 GB in 30s and still climbing |
| 0.6.4 | 0% | 123 MB, flat |

The content check that was doing this work is still there underneath. It
continues to dedupe a clipboard write that carries identical contents,
and on Windows and Linux, which have no equivalent counter, it remains
the only dedupe, exactly as before.

## Compatibility

No change to the history format, stored images, settings or licensing,
and nothing to migrate. What klipa records and when it records it are
unchanged: copying text, copying an image, and re-copying something
already in your history all behave as they did.

Pasting an image from history still has to hand the full picture to the
system clipboard, which is real work rather than wasted work, and klipa
returns those pages to the OS afterwards as it has since 0.6.3.

Keep awake, the menu bar clock, search and the Windows and Linux builds
are unchanged.
