# MAS closed-lid: candidate architecture and proof plan

Companion to `docs/mas-closed-lid-feasibility.md`. That doc establishes what
is possible and approvable. This doc is the concrete klipa implementation,
and the proof each claim still needs. It is a candidate requiring proof, not
a finished feature. Nothing here is installed on the Mac currently under the
direct-build physical test, to avoid contaminating that result.

## Scope of the MAS build

- Clamshell closed-lid (external display): native, sandbox-legal, shipping.
  Hold a `PreventUserIdleSystemSleep` assertion for the session and frame it
  truthfully when `clamshell.rs` reports `StaysAwake`/`NeedsPower`.
- Bare-laptop closed-lid: only when the user has installed the off-store
  Klipa Power Helper. The app detects it and invokes it; it never installs or
  escalates anything.

## Component 1: the MAS app (sandboxed, App Store)

- Detection: the helper is "present" iff the toggle script exists at
  `~/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect` (the
  one directory a sandboxed app may execute user scripts from). No probing of
  system paths.
- Invocation: run that script via `NSUserUnixTask` (Foundation, no entitlement
  beyond the sandbox) with argument `on`/`off`. The script, authorized by the
  user-installed sudoers rule, runs `sudo -n /usr/bin/pmset -a disablesleep
  1|0` and prints the read-back state. The app parses that, never shells out
  to `pmset` itself.
- State model: reuse the existing `ProtectionState` vocabulary (inactive /
  preparing / verifying / active / restoring / restoration-failed). "Active"
  only when the script's read-back confirms `SleepDisabled=1`. If the helper
  is absent, show an honest "install the Klipa Power Helper to keep awake with
  the lid closed on a bare laptop" with a link, never a dead or lying toggle.
- The app ships NO LaunchDaemon, writes NO file to a shared location, requests
  NO temporary-exception entitlement, and never escalates. Reviewer behavior
  is identical to customer behavior (no reviewer detection, no hidden paths).

## Component 2: the Klipa Power Helper (Developer ID, off-store, user-run)

Distributed as a separate notarized download from the klipa site, never
through the App Store. The user runs it once; it performs one native admin
authorization (the user, not the app):

1. Writes the toggle script to
   `~/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect`
   (`0755`, owned by the user). The script does only:
   `sudo -n /usr/bin/pmset -a disablesleep "$1"` for `1`/`0`, then
   `pmset -g | awk '/SleepDisabled/{print $2}'` as read-back.
2. Writes `/etc/sudoers.d/dev.peterdsp.klipa` (`0440 root:wheel`, validated
   with `visudo -cf` before install), scoped to exactly:
   ```
   <user> ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0
   ```
   and nothing else, ever.
3. Installs a per-user LaunchAgent
   (`~/Library/LaunchAgents/dev.peterdsp.klipa.powerprotect.plist`) that owns
   timed restore: for a timed closed-lid session the app records a deadline
   the agent reads, and the agent runs the toggle `off` at the deadline even
   behind a shut lid. This is the owner-chosen robust option for unattended
   timed expiry.
- Uninstall reverses all three and runs `pmset -a disablesleep 0`.

## Privileges and restoration behavior

- Privilege is a single sudoers NOPASSWD rule scoped to two exact commands.
  No daemon runs as root (unlike the direct build); the script runs as the
  user and elevates only those two `pmset` calls.
- Restore on stop/quit: the app runs the toggle `off` and verifies read-back.
- Restore on timed expiry behind a shut lid: the LaunchAgent owns it.
- Restore on crash/force-quit/reboot with no agent tick yet: best-effort on
  next login (a LaunchAgent `RunAtLoad` reconcile step checks a stamped
  deadline/owner file and restores if a session is no longer current).
- This is honestly weaker than the direct build's root daemon (which owns a
  lease, a root journal, and autonomous retry). The MAS companion's guarantees
  must be described as such, not equated with the direct build.

## Sandbox interaction

- Entitlements unchanged from today's MAS set: `app-sandbox`,
  `network.client`, `files.user-selected.read-write`, app/team identifiers.
- No app-group, no `mach-lookup`, no `temporary-exception.*`. The only
  cross-boundary action is executing a user script in the app's own
  Application Scripts directory via `NSUserUnixTask`, which the sandbox
  sanctions without extra entitlement.

## Review implications

- Guideline 2.4.5: compliant because the app installs nothing in shared
  locations (ii), downloads no code to add functionality (iv: the helper is a
  user-performed off-store install, not an in-app download), and never
  escalates to root (v: escalation is the user-installed sudoers rule, not the
  app). This mirrors Amphetamine's shipped pattern.
- Review notes must state plainly: closed-lid on a bare laptop requires the
  separately downloaded Klipa Power Helper; without it the app still keeps the
  Mac awake and reports clamshell state. Provide the reviewer the same helper
  download a customer uses.

## Proof plan: what must be shown, and what blocks it

| Claim | How to prove | Blocked on |
|---|---|---|
| `NSUserUnixTask` runs the Application-Scripts script from a sandboxed build | Run it from a MAS-signed build with the real entitlements | Apple MAS provisioning profile / signing (owner Apple account) |
| The sandboxed app cannot reach `/var/run` (so the direct socket path is truly out) | Attempt `UnixStream::connect` from the MAS-signed build; expect sandbox denial | Same MAS-signed build |
| sudoers rule + script toggle `disablesleep` 0<->1 with read-back | Install on a CLEAN test account/Mac, run the script, read `pmset -g` | A clean environment, NOT the Mac mid direct-build test (contamination) |
| LaunchAgent restores a timed session behind a shut lid | Physical timed closed-lid case on the clean env | Clean env + physical lid action (owner) |
| App Review accepts the shape | Submit the MAS build with accurate review notes | Owner Apple account; outcome is Apple's, not assertable in advance |

## Exact remaining owner actions for B

1. Confirm the direct build is physically verified and published first (agreed
   sequencing), so B testing happens on a clean, uncontaminated environment.
2. Provide Apple MAS signing access (Apple Distribution + 3rd Party Mac
   Developer Installer certs, App Store Connect app record) so a real
   sandboxed build can be produced and its sandbox behavior proven.
3. Run the physical closed-lid cases for the MAS companion on a clean account,
   without the direct helper installed and with no residual sleep override.

Until the MAS-signed build exists and these run, B stays a documented,
code-complete candidate, not a verified or approved feature. No "verified"
claim will be made for it before that evidence exists.
