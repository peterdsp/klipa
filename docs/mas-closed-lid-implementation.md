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

### Open sandbox dependency found during implementation (needs a signed build)

A sandboxed app's `~/Library/Application Support` is **redirected into its
container** (`~/Library/Containers/dev.peterdsp.klipa/Data/...`), while the
off-sandbox toggle script and the LaunchAgent watchdog run as the plain user
and see the **real** `~/Library/Application Support`. So the app and the
helper scripts do NOT automatically share a file there. The session stamp the
watchdog reads cannot simply be the app's `paths::data_dir()` file. Three
candidate resolutions, and which one is correct can only be settled on a real
MAS-signed sandboxed build:

1. **Script-owned state + argument passing (favored).** The app passes the
   verb and parameters (on/off/status, deadline, prior) to the toggle as
   `NSUserUnixTask` arguments, and reads the result from the task's captured
   stdout. The toggle (and watchdog) own the stamp at the real-path
   `~/Library/Application Support/dev.peterdsp.klipa/powerprotect.session`.
   The app never reads that file. The pure session logic already implemented
   (`powerprotect::SessionManager` and the stamp parse/serialize) then runs in
   the script side, with the Rust helpers as its tested specification and the
   app driving it by verb.
2. **Application Scripts directory as the shared path** (both see it at the
   real path), if the sandbox lets the app write there. Unverified.
3. **App-group container** shared between the app and a packaged helper. This
   reintroduces a helper the app ships, which pushes back toward the 2.4.5
   problems, so it is the least preferred.

This is the single most important thing to settle on a signed build; it does
not change the privilege model or the review posture, only where the shared
state lives and whether the session loop runs in Rust (app) or in the shell
(script).

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

## What is implemented and tested now (no device needed)

- Pure session + recovery logic in `crates/klipa-ui/src/powerprotect.rs`,
  over injected runner + stamp-store seams: engage/end/extend/expiry,
  prior-value capture and restoration (a user-preexisting `disablesleep=1` is
  marked unowned and never cleared), deadline validation, and launch
  reconciliation of a stranded owned session. 18 unit tests.
- The off-store helper (`packaging/macos/powerprotect/`): toggle, watchdog,
  sudoers template, LaunchAgent, install/uninstall. The sudoers rule passes
  `visudo -cf`; all scripts pass `sh -n`; the Rust `SessionStamp::to_text`
  output and the watchdog's awk parser are cross-checked to agree byte for
  byte.

## CI can build and App-Store-validate this branch (no new Apple access)

The repo already holds the MAS signing secrets (`MAS_CERTS_P12_BASE64`,
`MAS_CERTS_P12_PASSWORD`, `MAS_PROVISION_PROFILE_BASE64`, `ASC_*`, `TEAMID`) that
produced the 0.6.2 upload. `release.yml` now accepts
`workflow_dispatch -f mas_validate_only=true`, which builds the MAS pkg, signs
it with the Apple Distribution + 3rd Party Mac Developer Installer identities and
the provisioning profile, and runs `xcrun altool --validate-app` against the App
Store, WITHOUT uploading. So the signed-and-validatable MAS build of this branch
is produced in CI; no new owner Apple action is needed to get that far.

**Evidence (run 37224443856, branch `mas-powerprotect-candidate`, 2026-10-04):**
the `mas` job built the branch, `codesign`d it for the App Store, `productbuild`
signed `klipa-0.6.2-mas.pkg` with "3rd Party Mac Developer Installer: PETROS
DHESPOLLARI", and reached `altool --validate-app`. The ONLY validation error was
`-19232`: "The bundle version must be higher than the previously uploaded
version: '0.6.2'." That is a version collision, not a code/signing/entitlement
problem, and it independently confirms a 0.6.2 MAS build is already uploaded to
App Store Connect. The pipeline is proven; a clean validate (and any real
submission) just needs a higher version, once the Power Protect wiring and the
runtime sandbox checks below are done.

## Proof plan: what must be shown, and what blocks it

| Claim | How to prove | Status / blocked on |
|---|---|---|
| The MAS build compiles, signs (MAS cert + profile), and reaches `altool --validate-app` | CI `release.yml` dispatch with `mas_validate_only=true` | DONE (run 37224443856): built, signed, productbuilt, validated up to the version-collision check (-19232); pipeline proven, no new Apple access |
| `NSUserUnixTask` actually runs the Application-Scripts toggle from inside the sandbox, and stdout is capturable | Run a signed sandboxed build and observe | A RUNNABLE signed sandboxed build (TestFlight, or a Developer-ID + app-sandbox dev build); not just a CI-built pkg |
| Where app and scripts share state (the Application Support redirection fork above) | Observe real vs container paths on a signed sandboxed build | Same runnable sandboxed build |
| The sandboxed app cannot reach `/var/run` (direct socket truly out) | `UnixStream::connect` from the sandboxed build; expect denial | Same runnable sandboxed build |
| sudoers rule + toggle flips `disablesleep` 0<->1 with read-back, restores to prior | Install on a CLEAN account/Mac, run the toggle, read `pmset -g` | A clean environment, NOT the Mac mid direct-build test (contamination) |
| LaunchAgent restores a timed session behind a shut lid | Physical timed closed-lid case on the clean env | Clean env + physical lid action (owner) |
| App Review accepts the shape | Submit with accurate review notes | Owner Apple account; outcome is Apple's, not assertable in advance |

## The precise remaining dependencies for B (not "all of it")

Development that is NOT blocked is done (logic, companion, tests) or in CI
(signed MAS build + validate). What genuinely needs more than code:

1. A **runnable signed sandboxed build** (TestFlight build, or a local
   Developer-ID build carrying the `app-sandbox` entitlement) to settle the two
   runtime unknowns: that `NSUserUnixTask` executes the toggle and returns its
   output under the sandbox, and which state-sharing option (above) is correct.
   A CI-built MAS `.pkg` is for upload, not local run, so it does not by itself
   answer these.
2. A **clean test environment** and **physical lid actions** to prove the
   toggle/watchdog keep the Mac awake and restore, without contaminating the
   direct-build test.
3. The **owner Apple account** only for the final step: an actual App Store
   Connect upload + submission + review. Building and validating do not need new
   access; submitting for review does.

Until the runnable sandboxed build settles the runtime unknowns, B stays a
documented, logic-complete, CI-buildable candidate, not a verified or approved
feature. No "verified" claim will be made for it before that evidence exists.
