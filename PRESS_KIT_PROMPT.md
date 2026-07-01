# Reusable press-kit prompt

Paste this into a fresh Claude Code session on any project. Swap the
`{{placeholders}}` per project.

```
Build a real press kit for {{PROJECT_NAME}} at docs/press.html
(or wherever the site lives). Do NOT make it feel LLM-generated -
no "copy any of these", no "paste this or that", no multiple length
variants of the same paragraph. Treat it like a proper media kit
you'd find on Linear / Notion / Vercel / Arc.

Required structure, in order:

1. Hero
   - Small "Press / Media kit" eyebrow tag
   - "{{PROJECT_NAME}} press kit" heading
   - One-sentence lede
   - Primary CTA: "Download press kit (ZIP)"
   - Secondary CTA: mailto: press contact
   - Chip nav that jumps to About / Fact sheet / Media / Contact

2. About {{PROJECT_NAME}}
   - ONE clean boilerplate (1-2 paragraphs). Third person. No
     "here's a short version, here's a long version".

3. Fact sheet
   - Two-column dt/dd list: Product, Category, Platforms,
     Availability, Pricing, License, Privacy, Size, Stack, Website,
     Source, Store, Built by. Skip rows that don't apply.

4. Media & brand assets
   - Grid of tiles: each has thumbnail + name + format/dimensions +
     a "Download" link. One tile per screenshot, one per logo/icon
     variant. Full-res image is a click-through; the Download link
     hits the same file with `download` attr.

5. Press contact
   - Card: name, role, one-line availability note, email button.

Also generate a real ZIP at
docs/assets/{{project}}-press-kit.zip containing:
   - BOILERPLATE.txt       (the "About" paragraph, plain text)
   - FACT-SHEET.txt        (the fact sheet as an aligned table)
   - README.txt            (contents list + usage terms + contact)
   - All logo / icon variants at their native sizes
   - All full-res screenshots

Match the visual language of the rest of the site (steal colors,
radius, typography, blur, hover states from index.html - do NOT
introduce a new palette). Dark theme if the site is dark, light if
light. Keep sections airy: uppercase 12-13px letter-spaced section
labels, generous vertical rhythm, no dense grids of prose.

Constraints:
- Semantic HTML5 (header/section/nav/footer). Keyboard-navigable
  links. Alt text on every image. `loading="lazy"` on thumbs.
- No JavaScript needed for the page to work.
- Update meta/OG/Twitter tags for the press page.
- Don't touch the privacy policy unless a fact there is genuinely
  wrong now.

Before writing: skim the existing site (index.html or equivalent)
for the exact colors, radius, font stack, and existing screenshots
in docs/assets/. Reuse assets that already exist. If a required
asset is missing, list it at the end of your response - don't
invent placeholders.
```

## Notes when reusing

- Assumes a static site under `docs/` (GitHub Pages style). Swap the
  paths for Next/Astro/etc.
- The `BOILERPLATE.txt` + `FACT-SHEET.txt` + `README.txt` trio inside
  the ZIP is what makes the kit feel real vs. LLM-y. Keep them.
