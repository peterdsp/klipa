# Mac App Store closed-lid keep-awake: feasibility and architecture

Status date: 2026-10-03. Evidence-backed answer to one question the owner
asked us to stop assuming and start proving: **can the Mac App Store
(sandboxed) build of klipa keep a MacBook awake with the lid closed?**

The short answer corrects an imprecise blanket claim in the code and docs.
It is not true that "a sandboxed app can never do closed-lid." It is true
that a sandboxed app cannot do bare-laptop closed-lid *by itself*, and the
only route that works and is App-Store-approvable is the narrow
"user installs a privileged helper off-store" pattern that Amphetamine uses.

## The three mechanisms, compared

| Mechanism | What it needs | Works on bare laptop (no external display)? | Self-contained MAS? | Reliability of unattended timed expiry |
|---|---|---|---|---|
| IOPMAssertion (`PreventUserIdleSystemSleep`) | Public IOKit, no entitlement, no privilege | No. Defeats only the idle timeout, never an explicit lid close | Yes (ships today in every build) | N/A (does not hold a closed lid) |
| Clamshell (Apple's supported mode) | External display attached; +AC on Intel; Apple Silicon on battery | No (needs external display), but keeps a lid-closed Mac awake in that config with zero privilege | Yes (detected + reported today; see `clamshell.rs`) | Good (no override to restore) |
| Root `pmset -a disablesleep 1` | Root privilege | Yes. The only native lever for bare-laptop closed-lid | No. Needs an out-of-sandbox privileged component | Direct build: strong (daemon owns lease/journal/expiry). MAS companion: weaker (no daemon) |

## What a sandboxed MAS app CANNOT do (confirmed)

- **Connect to the direct build's `/var/run` Unix socket.** The App Sandbox
  blocks unmediated IPC between code from different teams; UNIX domain
  sockets are permitted only inside an app-group container, never a
  system path like `/var/run`. No entitlement fixes this
  (`temporary-exception.files.*` does not cover sockets). So the existing
  `klipa-helper` socket IPC is unreachable from a MAS build.
  Source: Apple DTS, https://developer.apple.com/forums/thread/788364
- **XPC to a root LaunchDaemon in a shippable MAS build.** The sanctioned
  API exists (`xpc_connection_create_mach_service`), but the client needs
  `com.apple.security.temporary-exception.mach-lookup.global-name`, which
  App Review rejects for privilege escalation under guideline 2.4.5(v).
  Source: Apple DTS, https://developer.apple.com/forums/thread/99602
- **Install the privileged component itself.** Guideline 2.4.5 forbids a
  MAS app from installing code in shared locations (ii), downloading code
  to add functionality (iv), or escalating to root (v). Writing a
  LaunchDaemon, or a `/etc/sudoers.d` file, from the app is a rejection.
  Source: https://developer.apple.com/app-store/review/guidelines/

## The one pattern that works (Amphetamine, confirmed)

Amphetamine is MAS-distributed and sandboxed, yet offers closed-display on
Apple Silicon. It does so without ever escalating in-app:

1. The **user** manually downloads and installs, off-store, two files:
   - a script into `~/Library/Application Scripts/com.if.Amphetamine/`
   - a sudoers drop-in at `/private/etc/sudoers.d/amphetamine_powerProtect`
   authorising passwordless `pmset`.
2. The sandboxed app then runs that user-placed script via the Application
   Scripts sandbox-escape (`NSUserUnixTask` / `NSUserScriptTask`, the one
   directory a sandboxed app may execute user scripts from). The script
   runs `pmset -a disablesleep` as root with no password (the sudoers rule).
3. No daemon, no socket, no XPC, no entitlement, no in-app root install.

Amphetamine's own docs attribute the manual install to Apple: it "won't
allow you to permit Amphetamine to directly install the script."
Sources: https://github.com/x74353/Amphetamine ,
https://github.com/x74353/Amphetamine-Power-Protect

The exact privileged artifact (from the clean-room reference
`imabee101/KeepAwake`, which pins the technique): sudoers file
`0440 root:wheel`, validated with `visudo -cf`, scoped to exactly two
commands and nothing else:

```
<user> ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0
```

Admin approval is obtained by a native prompt the **user** runs, never
headless. Source: https://github.com/imabee101/KeepAwake

We will independently implement the same technique (sudoers-scoped `pmset`
+ Application Scripts invocation). The technique is not proprietary; we copy
no third-party code.

## Corrected B architecture for klipa

Two components, matching the only approvable shape:

**Component 1, the MAS app (sandboxed, App Store).**
- Clamshell closed-lid, native and sandbox-legal: hold the existing
  `PreventUserIdleSystemSleep` assertion and frame it truthfully as
  closed-lid keep-awake *for external-display configs*. No privilege.
- Bare-laptop closed-lid: detect whether the user has installed the Klipa
  Power Helper (script present in
  `~/Library/Application Scripts/dev.peterdsp.klipa/`). If present, toggle
  `disablesleep` by running that script via `NSUserUnixTask`. If absent,
  show honest status and a link to the off-store helper + setup steps.
- The app NEVER installs the script or the sudoers rule, never escalates.
  Review notes describe this exactly.

**Component 2, the Klipa Power Helper (Developer-ID notarized, off-store,
NOT on the App Store).**
- A user-run installer that: writes the toggle script into the user's
  Application Scripts dir; writes `/etc/sudoers.d/dev.peterdsp.klipa`
  (`0440 root:wheel`, `visudo -cf` validated) scoped to exactly
  `/usr/bin/pmset -a disablesleep 1|0`; obtains one native admin prompt the
  user runs.

## Honest limitations of the MAS companion vs the direct build

- **Weaker unattended timed expiry.** The direct build's root daemon owns
  the lease, the recovery journal, and timed expiry, so a timed closed-lid
  session ends and restores behind a shut lid even if the app is gone. The
  MAS companion has no daemon: behind a closed lid the app cannot run to
  call the restore script. Mitigation to design: the off-store helper also
  installs a user LaunchAgent that owns timed restore, or the script
  self-schedules the restore. Until then, indefinite sessions are fine but
  unattended timed closed-lid restore is best-effort (restore on next
  launch / next wake), which must be disclosed, not hidden.
- **Residual App Review risk.** Approvable in the strict Amphetamine shape,
  but 2.4.5(v) is the hard bar; any drift toward in-app install/escalation
  is a rejection. This is a lead from Amphetamine's track record, not a
  guarantee for klipa.
- **Not superior to the direct build.** The direct download already gives
  robust bare-laptop closed-lid. The MAS companion exists only to serve
  users who prefer to get klipa from the App Store and will do the one-time
  off-store helper install.

## Bottom line

"Sandboxed means impossible" is false as a blanket; "self-contained
sandboxed bare-laptop closed-lid" is genuinely impossible. The strongest
viable MAS product is clamshell-native in-app plus an off-store,
user-installed `pmset`-scoped sudoers helper invoked via Application
Scripts, described accurately to App Review. The direct build remains the
primary, most robust channel for bare-laptop closed-lid.
