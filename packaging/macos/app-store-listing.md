# klipa — Mac App Store listing (v0.5.0)

Ready-to-paste copy for App Store Connect. Scoped to what the **sandboxed
App Store build actually ships**: clipboard history, keep-awake (idle)
sessions, the lid-close status, and the menu-bar date/temperature. It does
**not** mention the root lid-closed override or frontmost-app capture,
which are compiled out of the App Store build, and it omits the trial /
license copy, since the store handles payment.

Field character limits are noted; App Store Connect enforces them.

---

## Subtitle  (max 30 chars)

```
Menu bar clipboard history
```

## Promotional text  (max 170 chars — editable any time, no review)

```
A tiny, fast clipboard manager that lives in your menu bar. Your recent copies, one click away, and kept entirely on your Mac.
```

## Description  (max 4000 chars)

```
klipa is a tiny, fast clipboard manager that lives in your Mac's menu bar. Click the menu bar icon and your recent copies drop down, text and images alike. Click any entry to copy it back, ready to paste.

Everything stays on your Mac. Your clipboard history is kept in a single local file and is never logged, uploaded, or sent anywhere. klipa has no account, no sign-in, and makes no network connections for its core features.

FEATURES

- Clipboard history in your menu bar: text and images, with small image previews
- Click any entry to copy it back and paste
- Keep-awake sessions stop your Mac from sleeping, from 5 minutes to 5 hours, or indefinitely, so long downloads, renders, and presentations aren't cut short
- On Mac laptops, Keep awake also tells you at a glance whether closing the lid will keep your Mac running (with an external display connected) or send it to sleep
- Optional menu bar extras: today's date, the current temperature at your location, or both
- Adjust how many entries the history keeps and how many the dropdown shows

BUILT TO BE SMALL

klipa is written in pure Rust, with no Electron, no browser engine, and no JavaScript runtime. The whole interface is a native menu, so there is nothing to render. It is a small, self-contained app that stays light on memory.

PRIVATE BY DEFAULT

No tracking, no analytics, no cloud. Your clipboard never leaves your device. The optional temperature feature is the only thing that reaches the internet, and only when you turn it on.
```

## Keywords  (max 100 chars, comma-separated)

```
clipboard,menu bar,clipboard history,paste,copy,keep awake,caffeine,menubar,productivity,history
```

## What's New in This Version — 0.5.0  (max 4000 chars)

```
Keep awake now gives you a heads-up about the lid.

While Keep awake is on, klipa tells you what will happen if you close the lid:
- Stays awake — when an external display is connected (macOS clamshell mode)
- Mac will sleep — when there's no external display
- Connect power to stay awake — on Intel Macs running on battery

No more closing the lid and wondering whether your work kept running.
```

---

## Review-safety notes (do NOT paste — for you)

- The copy says klipa **shows / tells you** the lid outcome. It never says
  klipa keeps a Mac awake with the lid closed. The sandboxed build cannot
  do that (the only lever is a private Apple entitlement or root), so
  claiming it would risk a rejection. Clamshell running is macOS's own
  behavior and requires an external display.
- No trial / "EUR 1.99" text: the App Store version is a normal paid app;
  the store handles payment. That whole mechanism is compiled out.
- No "captures the app you copied from" claim: frontmost-window capture is
  disabled in the sandboxed build.
- Weather / date extras are included in the App Store build (it is built
  with the `weather` feature), so mentioning them is accurate.
- If the App Store "What's New" character count is tight, the short
  two-sentence variant also works: "Keep awake now shows whether closing
  the lid will keep your Mac running or send it to sleep, based on your
  external display and power. So shutting the lid never catches you off
  guard."
