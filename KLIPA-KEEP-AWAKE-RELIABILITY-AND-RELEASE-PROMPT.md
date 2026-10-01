# Klipa: finish every required fix, merge to main, release, deploy, and verify production

## Assignment

Work in `/Users/peterdsp/git/klipa`. Take complete ownership of finishing Klipa: implement every remaining requirement below, merge the verified work into `origin/main`, publish the new production release, update its distribution channels, deploy the public website, and verify the live result. The principal unresolved product problem is keeping a Mac running during an active keep-awake session, including with the lid physically closed and no external display. Investigate the causes, fix the architecture where necessary, prove the behavior, and finish delivery.

This is an implementation-and-delivery assignment. Do not stop at an audit, recommendations, documentation, mocked tests, a successful build, a local commit, an open PR, a version bump, a draft release, or a preview deployment. Work autonomously through implementation, verification, merging, packaging, publication, deployment, and production checks when the required gates pass. Preserve unrelated changes and existing clipboard functionality.

The user explicitly accepts substantial engineering, a properly installed privileged helper, native integration, and deeper investigation or reverse engineering when needed. Use those options to solve the actual behavior. Do not declare the problem impossible merely because ordinary idle-sleep assertions cannot prevent lid-close sleep, and do not repeat an unsuccessful `pmset` call more often as a substitute for finding the cause.

The product requirement is a temporary, explicit override of ordinary macOS sleep preferences during the requested session, followed by correct restoration. The direct-download macOS build is the primary delivery target for full closed-lid behavior. Keep the App Store build valid with accurately described capabilities; it must not limit what the direct build can achieve.

## Execution authorization and completion rules

I authorize the necessary code and workflow changes, tests, versioning, commits, pushes, PR creation, conflict resolution, merging into main, tag creation, signed builds, notarization, publication of the verified release, package-manager submissions, and deployment to Klipa's existing production website. This authorization applies when this prompt is submitted for execution. Do not ask again whether I want the work merged, released, or deployed after the checks pass.

Follow real branch protections, required reviews, OS consent, environment approvals, and account access requirements. Do not bypass them or claim they passed. Older project notes saying the owner must manually publish are not a new product requirement: with this authorization, perform that publication yourself once the evidence and actual permissions allow it. Retain all verification gates.

Ask only for genuinely necessary authentication, OS approval, unavailable physical interaction, a required reviewer, or a consequential ambiguity that cannot be resolved from the repository. Use existing configured identities and secret references; never print secret values, request them in chat, or commit them. Check access early so credential issues do not appear only at the end.

Do not return another menu of “push / hold / redesign” choices. The chosen direction is to complete the deeper redesign and deliver it. Target `0.6.0` if it is still the next appropriate unused version; refresh releases/tags and advance normally if that version already exists. Do not ship an intermediate `0.5.5` as the completed solution while requirements remain deferred.

Physical lid testing gates stable release claims and publication. It does not excuse leaving implementable helper, session, UI, updater, test, merge, or packaging work undone. Prepare the test harness and packaged candidate, then request only the specific lid/power actions you cannot perform. Continue all independent work while waiting. Never invent a physical test result or treat elapsed time as user confirmation.

Keep a durable requirement checklist linking each item to implementation, test evidence, and release/deployment status. Record a fix as complete only when its acceptance criterion passes. If context becomes short, save the exact checkpoint and continue from it. If an external dependency blocks one channel, finish other authorized channels and report the outstanding channel accurately.

The work is done when the required fixes are on remote main, the verified signed release is publicly downloadable, the update and package-manager paths work, and the production website is deployed and checked. A real unmet external gate means the overall task remains explicitly incomplete; it does not permit a substitute “done” claim.

## 1. Establish the baseline and reproduce the problem

Read applicable repository instructions, inspect the current checkout and installed app, and refresh remote tags/releases before selecting the new version. Revalidate local and remote state before changing anything.

At the latest local inspection on 2026-10-01, `main` was five commits ahead of the local `origin/main` tracking reference, with workspace version `0.5.4`. The five commits were:

| Commit | Existing work to preserve and verify |
| --- | --- |
| `fb229cf` | Fix pre-existing clippy lints. |
| `9094606` | Prior-value-aware restoration and an app-side durable recovery journal. |
| `4176f17` | Bound helper request size/time and test the command whitelist. |
| `7b3589b` | Harden CI and release workflows. |
| `45fc712` | Document investigation, verification, and release gates. |

The original investigation baseline was `368cf36`. These commit observations are a checkpoint, not proof the remote is unchanged or every existing patch is correct. Fetch current remote state, inspect the five patches and current tests, keep useful work, fix defects, and complete the remaining design. This prompt file was untracked at inspection; preserve it and commit it if appropriate for durable project guidance.

Read `docs/keep-awake-investigation.md`, `docs/keep-awake-verification.md`, and `docs/keep-awake-release-checklist.md` as prior evidence. They explicitly list unfinished helper authentication/ownership/recovery, richer verified UI state, post-trial controls, transaction continuity, and updater handover. These are required work in this assignment. Reconcile inaccuracies between those documents, code, and tests; documentation cannot certify its own claims.

Inspect these existing files first, then follow their dependencies:

- `crates/klipa-ui/src/awake.rs`: modes, `KeepAwake`, `WakeLock`, assertions, privileged fallback, recovery marker, expiry.
- `crates/klipa-ui/src/helper.rs`: service registration, helper state, socket client.
- `crates/klipa-helper/src/main.rs` and `build.rs`: privileged daemon, protocol, embedded helper identity.
- `crates/klipa-ui/src/clamshell.rs`: hardware/power/display detection and predicted lid behavior.
- `crates/klipa-ui/src/main.rs`, `tray.rs`, `prompt.rs`, `settings.rs`, `paths.rs`: session controls, event loop, preferences, status, recovery, quit.
- `crates/klipa-ui/src/updater.rs`: bundle replacement, helper compatibility, relaunch.
- `Cargo.toml`, `Cargo.lock`, crate manifests, `packaging/macos/`, packaging scripts, `.github/workflows/ci.yml`, `.github/workflows/release.yml`.
- `README.md`, `docs/lid-closed-keep-awake.md`, public website/welcome/press text, App Store listing, package-manager manifests.

Record the actual failing environment: machine model, hardware architecture and process architecture, macOS version/build, installed app path/version/channel/signature, helper version/registration/approval/liveness, selected mode/duration, charger state, displays/docks, lid state, relevant sleep settings, and other active inhibitors. Do not infer that the app currently installed is built from this checkout.

Reproduce before and after on the same hardware/settings where possible. Collect timestamped, read-only power diagnostics, for example `pmset -g`, `pmset -g custom`, `pmset -g assertions`, relevant `pmset -g log` entries, bounded relevant unified logs, and native power/lid events. Redact identifying information and keep clipboard contents out of diagnostics. Use a bounded capture window rather than copying entire machine logs.

Do not change the host's global power configuration until its baseline and restoration path are recorded. Coordinate physical lid closure, unplugging, logout, or reboot with the user if the environment cannot perform them. Continue independent development while waiting for those physical steps.

### Concrete code findings to investigate and fix

These were observed in the original `368cf36` source, not proven as the cause of every reported sleep event. Some have since received partial fixes in the five commits above. Reconfirm each against the current checkout, credit verified fixes, and close remaining gaps without needlessly rewriting correct work:

1. macOS `Lock` implements the default `WakeLock::finished()` behavior, which always returns false. After engagement, there is no native assertion-health or override-health reconciliation. A `Session` object is treated as evidence of continuing protection.
2. `sleep_currently_disabled()` reduces command errors, unreadable output, and an actual false value to the same boolean. It does not require successful command exit. Unknown state can therefore be mistaken for successful restoration.
3. `Lock::drop()` and `recover_lid_closed()` ignore restoration failures and clear the marker anyway. Failed restoration can lose the evidence needed for retry.
4. The override always writes `disablesleep` back to zero rather than restoring the observed prior value. The marker is written after changing the setting, is only a boolean, and ignores write errors.
5. The root helper accepts unauthenticated commands on a mode-0666 socket. It handles one connection at a time with an unbounded `read_line` and no server read deadline; a stalled client can block all requests. It has no session ownership, deadline, lease, durable journal, or autonomous recovery.
6. Helper `Active` is derived from service registration, not authenticated liveness, protocol compatibility, or a verified operation. Registration and uninstall errors are largely logged rather than represented in the UI.
7. `KeepAwake::start()` calls `end()` before acquiring the replacement. Updating a duration or switching modes can temporarily remove protection, restore system sleep, and request authorization again. With the lid already shut, even a brief gap matters.
8. Privileged subprocesses, verification polling, and release in `Drop` can run on the menu event-loop thread. Expiry depends on that loop; modal UI or a stalled operation can postpone it.
9. The updater swaps the bundle and calls `std::process::exit(0)` from a background thread. That bypasses Rust destructors and the normal `awake.end()` path. It also replaces an app containing a registered helper without a verified helper handover.
10. `clamshell.rs` identifies a laptop using the active built-in display and assumes Apple Silicon plus an external display implies battery clamshell readiness. An inactive internal panel is not evidence that the machine has no lid. Architecture alone is not sufficient proof of a supported power/display configuration.
11. The tray labels an override rejection as a managed-power-policy issue without establishing that cause. Other labels assert success from local session state.
12. The trial-expired menu omits keep-awake controls. Audit whether an active session or pending restoration becomes inaccessible when licensing state changes. Ending protection and restoring settings must remain available.
13. The release workflow permits unsigned macOS artifacts, ignores Gatekeeper failure with `|| true`, and does not make its publish job depend on the separate CI workflow passing for the exact release commit.

Add any further issues demonstrated by inspection or reproduction. Do not spend time preserving assumptions in old comments that prevent the correct design.

## 2. Define and prove the mechanism

Distinguish display idle sleep, system idle sleep, lock screen, physical lid sleep, explicit user-requested sleep, dark wake, power-source transitions, and emergency sleep/shutdown. Treat them as different behaviors in diagnostics and tests.

Use current Apple documentation, installed SDK headers, version-relevant Apple open-source power-management code, and reputable projects' own source or documentation. Inspect mechanisms and observe behavior where public documentation is incomplete. Record source revision, OS version, experiment, result, and limitations. Preserve required attribution for reused code.

Useful primary-source starting points:

- [Apple: PreventUserIdleSystemSleep](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventuseridlesystemsleep): this idle assertion still permits sleep for reasons including lid closure, Apple-menu sleep, and low battery. It is not a closed-lid solution by itself.
- [Apple: PreventUserIdleDisplaySleep](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventuseridledisplaysleep): holding the display against idle sleep also prevents system idle sleep under its documented conditions; it does not wake an already-off display by itself.
- [Apple: PreventSystemSleep](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventsystemsleep): investigate its distinct semantics and applicability in the current SDK/OS before proposing it as a solution.
- [Apple: sleep/wake notifications, QA1340](https://developer.apple.com/library/archive/qa/qa1340/_index.html): useful historical API background, to be checked against current behavior.
- [Apple PowerManagement source](https://github.com/apple-oss-distributions/PowerManagement) and [Apple IOKitUser source](https://github.com/apple-oss-distributions/IOKitUser): investigate assertions, settings, and transitions in version-relevant source.
- [Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice): modern bundled service registration on macOS 13 and later, subject to user approval.
- [Amphetamine Enhancer](https://github.com/x74353/Amphetamine-Enhancer): the developer describes a separate helper providing a closed-display fail-safe. This does not prove Klipa's implementation or universal hardware support.
- [Amphetamine Power Protect](https://github.com/x74353/Amphetamine-Power-Protect): the developer specifically documents Apple Silicon closed-display problems after connecting/disconnecting external power. Investigate this transition class; do not assume a one-time successful setting write survives it.
- [Apple M3 closed-lid dual-display setup](https://support.apple.com/en-sg/117373): the documented setup includes power. Do not generalize battery support from architecture alone.

Compare candidate approaches in a short engineering decision record: public assertions, the existing privileged `disablesleep` approach, and any additional native mechanism justified by research. Evaluate actual continuous workload execution with the lid shut, power transitions, restoration, helper crashes, OS compatibility, privileges, and distribution constraints.

Do not retain broad claims such as “every app uses this one flag,” “all Apple Silicon Macs behave this way,” or “no App Store app can ever do this” without direct supporting evidence. Likewise, do not promise an undocumented technique works on every Mac just because it works on one machine.

Prefer the mechanism that passes the required behavior with the least persistent global mutation. A native bridge or a more capable helper is acceptable. Private or undocumented behavior, if essential to the direct build, must be isolated, detected at runtime, version-tested, observable, and recoverable. Keep it out of the App Store binary where not permitted. Do not make users install a competing utility.

The ordinary product must run with SIP, Gatekeeper, and normal boot security enabled. Global security changes or kernel patching are not a shippable fix for this utility. OS-enforced thermal protection, critical battery behavior, shutdown, and enforced management restrictions are not ordinary sleep preferences; report such constraints accurately rather than defeating them or falsely claiming protection.

## 3. Implement an explicit session contract

Preserve the three clear user intents:

| Mode | Required behavior |
| --- | --- |
| Keep screen and Mac awake | Prevent idle display and system sleep while active; respect lock/authentication policy. |
| Keep Mac awake, allow screen off | Background work continues while the display follows its normal sleep behavior. |
| Keep running with lid closed | Background work continues with the physical lid closed, including without an external monitor, on the advertised and tested power configurations. |

The primary failing Mac's closed-lid scenario must pass. Reclassifying that scenario as unsupported or hiding the control does not complete this assignment. If a real platform constraint remains after investigation, preserve the evidence and explicitly report an incomplete objective instead of publishing a “fully fixed” claim.

Support the existing presets, custom durations, and genuine indefinite sessions. Indefinite means no arbitrary session-expiry ceiling. A renewable ownership lease used for crash safety is a different concept and must not silently cap a healthy indefinite session.

Persist mode preferences, not an automatically resurrected session after a normal app restart or reboot. Define update handover separately. Ending a session normally restores eligibility for sleep; it does not promise immediate forced sleep despite other apps' assertions. Verify actual post-expiry sleep in a controlled baseline without competing inhibitors.

Replace boolean success with a state model that distinguishes at least inactive, preparing, awaiting approval, verifying, active, degraded/recovering, restoring, and restoration failed. Include requested behavior, effective verified capabilities, ownership, deadline, and a concrete last error. Never show “kept awake by Klipa” solely because an in-memory session exists or a subprocess returned zero.

Serialize state transitions. Use operation/session identifiers so delayed replies from an old start, stop, or helper instance cannot change a newer session. Starting/stopping and retries must be idempotent.

Change durations in place where possible. When changing mechanism or mode, acquire and verify replacement protection before retiring old protection, or use a demonstrably gap-free transaction. A bounded overlap of owned assertions is acceptable if needed for continuity; the existing “exactly one assertion at every instant” rule must not create a real sleep gap. Ensure every handle has an owner and is eventually released.

Use a clock whose elapsed-time behavior across real sleep is understood and tested. Timed sessions must not gain extra time after a forced sleep, clock adjustment, UI stall, or helper reconnect. Handle boot identity separately from within-boot continuous time. Use checked duration arithmetic.

Explicit stop/quit must reliably release Klipa's protection. If a chosen system override affects Apple-menu Sleep, describe that effect accurately and provide an accessible stop-and-sleep route if needed. Never use fake keyboard/mouse activity to defeat lock-screen authentication.

## 4. Give the helper real ownership and recovery responsibilities

If privileged changes are necessary, make one trusted component own the transaction from acquisition through verified restoration. Prefer a structured, versioned, authenticated IPC protocol, such as a narrowly scoped XPC service with audit-token/code-signature validation. An alternative transport must enforce equivalent caller identity and request bounds; checking only a PID, claimed bundle name, or world-writable socket access is insufficient.

Implement narrowly scoped operations for capability/status, begin, update duration/mode, renew/reconnect, end, and recovery. Return structured results, including helper/protocol version, session generation, current effective state, and specific failure reason. Do not accept arbitrary shell commands, executable paths, settings keys, or environment variables from the client.

Authenticate both endpoints where appropriate. Validate the actual signed app identity/Team ID and authorized user/session context. Handle multiple app instances and users deliberately. Removing the helper or stopping one client's session must not erase another valid owner's protection. Development-only signing accommodations must not weaken the production daemon.

Bound message size, read/write time, subprocess time, concurrency, queue size, retries, and log growth. A client that connects and sends nothing must not block all power operations or restoration. Never call an interactive password prompt from the daemon.

For settings that must be changed, record the exact affected scope and original values in an atomic, root-owned recovery journal before mutation. Determine whether a setting is global or per-power-source rather than assuming `-a` semantics. Include journal schema, ownership, boot identity, session generation, intended change, transaction phase, and appropriate deadline/lease metadata. Do not proceed if required durable recovery state cannot be written.

Restore only changes Klipa owns, verify the result, and remove recovery records only after successful reconciliation. An existing `disablesleep=1` must not be blindly reset to zero. Avoid whole-file power-preference replacement. Detect external changes where possible and define a visible conflict policy; a matching boolean alone cannot prove exclusive ownership against other utilities.

Choose and document client-loss behavior: ordinary app quit/crash/force-quit ends its session and the helper restores within a small measured grace period. The helper owns timed expiry independently of the UI. Keep lease renewals independent of menu tracking and modal dialogs. Do not make a frozen UI capable of leaving a global override stuck indefinitely.

On helper crash/restart, interrupted start/stop, or boot recovery, reconcile the journal before accepting normal work. Use launchd recovery and a bounded fail-safe appropriate to the selected mechanism. Never depend solely on Rust `Drop`, which cannot run after `SIGKILL`, abort, or power loss. Test interrupted writes and interrupted setting changes.

No fresh admin prompt may be required when a timed session expires with the lid closed. If that cannot be guaranteed by the current one-shot `osascript` fallback, replace it with one-time helper setup or another proven unattended restoration design. Do not offer that fallback as reliable timed closed-lid support.

Migrate older installations deliberately. The old `lid_awake.on` marker contains no trustworthy original-value snapshot; do not fabricate one. Retain unresolved legacy recovery evidence and use an explicit recovery flow where ownership is ambiguous. Remove or disable the legacy unauthenticated endpoint when the new service takes over, and verify old/new protocol mismatch behavior.

Helper removal must first end/reconcile owned sessions and restore settings. Do not unregister a service while it is still the only component able to undo an active override. Deal explicitly with OS-level approval revocation, logout, moved/deleted bundles, and unavailable helpers; report any unavoidable recovery limitation instead of hiding it.

`SMAppService` is a macOS 13+ path. The repo advertises macOS 11+. Either implement and verify an appropriate legacy mechanism or explicitly gate only the advanced feature on older systems while preserving ordinary keep-awake there. Verify weak-linking/runtime availability and deployment targets; a runtime version check alone does not prove an older OS can load the binary.

## 5. Monitor real state without blocking the UI

Subscribe to appropriate native power-source, sleep/wake, lid, display, and helper lifecycle notifications. Do not assume winit's `resumed` callback is a complete macOS power notification system. Combine events with a modest background health check where events cannot prove continuity.

Reconcile before declaring a session active and after charger/dock/display changes, unlock/wake, helper reconnection, and relevant settings changes. Reacquire invalid assertions and reapply an owned override when appropriate, with bounded retries. Known power-source races need a tested transition strategy; repairing a setting after the Mac already slept does not count as preventing that sleep.

Represent enabled, disabled, unknown/unreadable, and failed observations separately. Prefer structured native state where available. If parsing `pmset` is necessary, validate exit status and expected fields, handle version/output differences, and preserve diagnostic evidence. Do not infer MDM merely from failure to apply a value.

Check ownership/effective assertion state using appropriate native APIs, not just the presence of a local handle. A flag or assertion readback is necessary control evidence, not proof that a closed-lid workload actually ran continuously.

Move blocking IPC, subprocesses, polling, and recovery off the main thread. Return results to the event loop for rendering. Bound all external operations. `Drop` may provide last-resort cleanup, but fallible asynchronous restoration needs an explicit stateful operation with a visible result.

Separate device identity, physical lid state, active displays, power source, supported capabilities, and current override state. Do not label a MacBook as a desktop because its internal display is inactive. Distinguish virtual displays from physical monitor support where relevant. Present uncertain clamshell predictions as uncertain.

## 6. Make the existing menu truthful and usable

Keep Klipa's native menu-bar product and existing clipboard workflows. Put understandable state and actions in the existing Keep awake menu:

- Selected mode, actual active duration/indefinite status, and verified protection state.
- One-time helper setup/approval instructions only when needed, including a retry after approval.
- Specific recovery actions for unavailable/incompatible helper, denied authorization, lost protection, and failed restoration.
- An always-reachable End session / Restore normal sleep action, including after trial expiry or activation failure.
- A concise explanation when the current build/hardware lacks a requested capability.
- A copy/export diagnostics action containing relevant app/helper/power state and recent bounded events, without clipboard contents, credentials, or unnecessary identifying data.

Do not equate service registration with readiness. Do not silently downgrade closed-lid protection to idle-only mode while presenting success. Separate preference selection, pending setup, and effective behavior.

Keep sensible thermal and critical-battery handling and a short first-use closed-lid explanation. Preserve system emergency protection; do not claim a closed lid prevents all cooling. Ordinary sessions must not become a repeated warning or password-dialog workflow.

Update README, the lid document, release notes, website/welcome/press copy where affected, and App Store descriptions to match verified behavior and distribution differences. Remove categorical unsupported claims from the old documentation. Preserve existing languages and branding.

## 7. Test the actual failure modes

Also perform a focused regression pass over the product's existing critical paths: first launch, text/image clipboard capture, history persistence and copying, clear-history behavior, licensing/trial transitions, settings persistence, menu controls, optional weather failure/offline behavior, and update discovery. Fix demonstrated regressions and release blockers found in scope. Turn “everything fixed” into a tracked set of observed defects and requirements; do not claim to have proved the absence of every possible bug.

Add meaningful deterministic tests around the session state machine, clock, helper protocol, journal, backend observations, and interrupted transactions. Use injected clocks and backends instead of long sleeps in unit tests. Cover at least:

- Acquisition success/failure, unknown readback, rollback failure, and retained recovery state.
- True indefinite duration, custom-duration bounds, elapsed-time behavior, deadline replacement, and no transient unprotected gap when changing duration or mode.
- Charger/display/lid transitions, assertion invalidation, helper loss/reconnect/version mismatch, stale replies, cancellation during start, and concurrent stop/start.
- Exact prior-value restoration, pre-existing override, multiple clients, external changes, journal corruption/unwritable storage, and crashes at transaction boundaries.
- Timed expiry while the menu is open or a modal is displayed; client heartbeat independent of UI work.
- Authentication rejection, oversized/partial/stalled requests, timeouts, replay/stale session operations, and graceful behavior under request saturation.
- Upgrade handover, legacy recovery, helper removal, failed restoration, and keep-awake controls remaining available after trial expiry.
- Direct-download and App Store feature separation, plus Windows/Linux behavior affected by shared session changes.

Run the repository's required checks and at least the applicable equivalents of:

```sh
cargo fmt --all -- --check
cargo build --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p klipa-ui --no-default-features --features "mas weather" --locked
cargo test -p klipa-ui --no-default-features --features mas --locked
cargo test -p klipa-ui --no-default-features --locked
```

Exercise feature combinations on macOS, and platform-specific work on the corresponding OS. Update commands if architecture changes justify it, recording the reason. Clearly distinguish pre-existing unrelated failures from introduced failures. Ensure CI tests the new helper logic instead of only compiling the daemon.

### Physical-device release gate

Use a real MacBook and the actual packaged candidate, with the same app/helper signatures and paths users receive. A VM, injected lid event, CI runner, successful assertion creation, screenshot, or `SleepDisabled=1` alone cannot satisfy the closed-lid gate.

Before testing, inventory other keep-awake tools, remote sessions, debugger assertions, and baseline inhibitors. Ensure the observer and test workload do not themselves prevent the sleep under test. Do not rely on continuous SSH/interactive input as proof: it can alter the result. Establish a control run showing the test machine sleeps under the same idle conditions with Klipa inactive.

Use a local timestamped heartbeat/work-progress recorder that takes no power assertions, plus sleep/wake logs and lid/power observations. Measure cadence and gaps against the inactive baseline. Add a representative download/build/server workload, but retain a low-activity control to ensure CPU or network activity did not mask a failure. Record dark wake distinctly; intermittent wake periods are not uninterrupted work.

Test these cases, recording exact hardware, OS build, artifact hash, helper version, starting settings, elapsed time, observations, and result:

| Case | Required evidence |
| --- | --- |
| Open lid, short system idle timeout | Each ordinary mode prevents the intended idle sleep for longer than the baseline timeout. |
| Display sleep and lock screen | System-only mode continues work with the screen off; lock/unlock remains functional. |
| Closed lid, no external display, AC | At least 30 minutes of continuous work and no unintended system sleep. |
| Closed lid, no external display, battery | At least 30 minutes where battery operation is advertised, with enough starting charge; no emergency-condition test by draining the battery. |
| Charger connect/disconnect while shut | Repeat at least five transitions in both directions; no unintended sleep or misleading active state. |
| Dock/external-display changes | Connect/disconnect and power-delivery transitions; protection and status reconcile correctly. |
| Repeated lid cycles | At least ten open/close cycles during a session without loss of protection. |
| Short timed session, lid shut | Ends unattended; owned settings restore within a stated, measured tolerance; sleep becomes possible again. |
| Timed session with UI occupied | Menu held open/modal displayed cannot delay helper expiry or strand the override. |
| Indefinite session | At least a two-hour soak on suitable power; no invented expiry or accumulating assertions/processes. |
| Duration and mode changes while shut | No intermediate sleep gap; deadlines do not reset unintentionally. |
| Normal quit and force-quit | Helper restores without needing Klipa to relaunch. |
| Helper crash/restart and failed restore | Recovery survives and the UI does not declare success prematurely. |
| Reboot/logout and service approval changes | No unexplained persistent override or silently resurrected session; disclose and resolve any recovery boundary. |
| Actual update from the old installed version | New bundle/helper run, settings/history/license survive, no stranded sleep override. |
| Stop / restore / helper uninstall | Original values and normal baseline sleep behavior return; no orphan service or endpoint. |

Use the user's failing hardware first, then Intel and Apple Silicon coverage for every architecture-wide claim. Track macOS versions supported for ordinary and advanced modes separately. Do not generalize cross-compilation into real hardware verification. Additional unavailable combinations remain explicitly untested; do not silently narrow the original requested scenario to get a pass.

If physical hardware access or a user-operated lid step is unavailable, finish the code, automated checks, signed candidate where possible, and an exact short test handoff. Keep the release gate incomplete and state precisely what evidence is missing. Do not publish a stable “fully working closed-lid fix” based on simulated success.

## 8. Make installation and updating part of the fix

Verify clean install, upgrade from the current released app, existing helper migration, missing/revoked approval, non-admin user behavior, and a moved application. App and helper identity/version/protocol must agree. Setup errors must be visible and recoverable.

Refactor the macOS updater so downloading, verification, session coordination, bundle replacement, helper transition, relaunch, health verification, and rollback form an explicit sequence. Do not let a detached worker call `process::exit` before owned power state is reconciled.

Before accepting an update, verify the expected release/version, trusted app identity/Team ID, nested helper signature, integrity, and applicable Gatekeeper/notarization state. Use safe private staging and a real rollback path. Checksums detect damage; they are not a replacement for authenticated code identity. Do not delete the previous bundle before the new app/helper is healthy.

Choose a tested update policy for active sessions: authenticated continuous handover or an explicit, verified stop before replacement. Do not silently drop a user-requested session or claim it continues if it was stopped. Preserve history/preferences/license without weakening helper authentication to make the upgrade pass.

Provide a clear normal uninstall/recovery path. The old app's update behavior can itself skip cleanup, so test the real old-to-new path rather than only new-to-new upgrades.

## 9. Merge, package, release, and deploy the verified work

### Land the complete implementation on main

Inspect the current remote, branch protections, required checks, and existing PRs. Use a `codex/` implementation branch where appropriate and carry the five existing commits forward without losing them. Prefer the repository's normal reviewed PR path; if direct fast-forward pushes to main are permitted and appropriate, they are authorized. Do not force-push main, discard unrelated work, disable protections, or manufacture reviewer approval.

Create or update a concrete PR containing the final implementation, relevant tests, migration, workflow changes, and documentation. Describe the final behavior and real validation. Attach created PRs to the chat when the environment supports it. Resolve conflicts, address actionable findings, wait for the required checks, and actually merge when the merge requirements are satisfied. An open PR is an intermediate state.

Verify that `origin/main` contains the complete intended diff after merge, including squash/rebase outcomes. Run required validation for the resulting main commit; do not assume a pre-merge result certifies a different source tree. Required follow-up fixes go through the same process. Keep the source version coherent with the planned release.

Physical verification may run against the merged candidate before public release. Keep unreleased features accurately described, and do not let a source merge publish an unverified stable binary or advertise a fix as available before its release. Inspect Pages publishing configuration before pushing documentation: a main-branch push can publish website content independently of the release workflow.

### Build and promote one identifiable release candidate

Harden the existing scripts/workflows instead of bypassing them:

1. Tie version, source commit, lockfile, app, helper, bundle metadata, release notes, and expected package-manager updates together. Do not reuse or overwrite a published version.
2. Require required CI checks to pass for the exact candidate commit on remote main. Include formatting and feature checks required by this prompt; the existing verify-job comment mentions formatting, but the inspected job did not execute that check. Resolve such mismatches. A green build on another commit is insufficient.
3. Build the supported macOS architectures and validate app/helper slices and deployment targets. Sign nested code inside out with the intended identities and hardened runtime.
4. Require Developer ID signatures, accepted notarization, valid stapling where applicable, and passing artifact/Gatekeeper checks for the public direct-download channel. Unsigned/ad-hoc artifacts may exist as clearly identified developer artifacts, but cannot reach the public stable release. Remove swallowed validation failures from the release path.
5. Validate both the installer channel and the separately distributed update ZIP/app. Inspect the actual embedded helper, launchd plist, designated requirements, entitlements, and versions.
6. Use appropriate checks including `codesign --verify --strict`, explicit nested-helper verification, `pkgutil --check-signature`, `spctl` assessment for the installed app/installer, `xcrun stapler validate`, and architecture/metadata inspection. Confirm actual notarization status, not merely command invocation.
7. Stage release assets without public announcement, run the physical/install/update gates against the final candidate, and retain the tested artifact hashes. Publish those bytes; if the candidate is rebuilt or behavior-affecting inputs change, renew the affected gates.
8. Fix the workflow's ordering so pushing a tag cannot automatically publish unverified artifacts. A draft release or equivalent staged promotion is acceptable. Bind the gate record to the candidate commit, artifact hashes, signing/notarization evidence, and hardware-test results. A document claiming checks passed is not enforcement. After actual gates pass, publish the release yourself under the authorization above; do not stop merely because the workflow stages a draft. Reject attempts to overwrite assets on an already published release, including reruns of the current `--clobber` upload path. Reruns must be safe and idempotent.
9. Preserve Windows/Linux release outputs and regression checks. Keep the MAS package separate, sandboxed, and free of disallowed helper/private behavior. If existing App Store credentials permit upload, validate/upload through the established channel; distinguish upload, processing, review, and public availability. Missing MAS access need not falsely mark the direct-download release incomplete, but must be reported separately.
10. After all required gates pass, create/push the new tag from the verified commit on main and publish the release through the corrected process. A tag created earlier solely to stage a draft must already identify that same final commit; never move a published tag. Verify every expected public asset, checksum, version, release URL, and downloadable installer/update ZIP. Set the intended stable release as latest and verify the latest-release API used by Klipa returns it. Download and inspect the published macOS artifact and smoke-test installation/launch/helper status from that artifact.
11. Update Homebrew/Scoop and applicable winget/AUR manifests from actual published artifact checksums. Validate the updater finds the new version, website download links resolve correctly, and public copy matches the tested support matrix. Report external submission/publication states separately.
12. Prepare recovery/rollback instructions for a release regression, including restoring owned sleep changes and restoring app/helper compatibility. Preserve published version identity: use an explicit withdrawal/latest-selection policy or a new corrective version instead of silently replacing old release assets with different bytes. Verify updater and manifest behavior for the chosen recovery action. Never depend on removing a user's unrelated power settings.

### Verify publication triggers and complete package-manager delivery

The current workflow updates manifests on `release: published`. Confirm the authentication and event chain actually starts those jobs when the agent publishes. GitHub documents that `GITHUB_TOKEN`-generated events generally do not create new workflow runs, with specific exceptions; do not rely on a token-created publication to automatically trigger a second workflow. Use a suitable authorized credential, an explicit supported dispatch, or dependent/reusable jobs, and verify the resulting run. See [GitHub's token-trigger rules](https://docs.github.com/en/actions/concepts/security/github_token).

Wait for package-manager updates to finish and verify their actual contents. If protections prevent a bot push to main, create and merge the manifest PR through the required checks. Main must contain the final manifest changes, not leave them only in a workflow artifact or unmerged branch. Hashes must match public assets. Validate the Homebrew cask and Scoop manifest, and complete configured winget/AUR submissions where access permits. Distinguish submitted from externally accepted; do not claim a third-party maintainer merged something without evidence.

### Deploy and verify the production website

The repository contains a static website under `docs/` and `docs/CNAME` contains `klipa.peterdsp.dev`. Inspect the live repository hosting configuration and deployment history to identify its actual production mechanism. Do not invent a new host, replace the domain, or assume that the absence of a Pages YAML file means there is no deployment. GitHub Pages supports both branch/folder publishing and custom workflows; see [GitHub's publishing-source documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site).

Preserve the domain, HTTPS, branding, assets, languages, and existing working pages. Update affected product descriptions and availability claims to match the new tested release. The inspected home page resolves download assets through GitHub's latest-release API: verify asset selection, missing-asset/error behavior, and stable-release selection. Provide usable download navigation when API loading fails; do not leave actionable download buttons pointing at `#`.

Deploy through the existing production process and wait for the deployment associated with the intended commit to complete. If configured permissions or workflow dispatch need repair, implement the minimum repository-appropriate fix and verify it. Account for bot pushes that do not start branch-based Pages builds; a successful manifest commit is not proof the website deployed. Do not modify unrelated DNS or hosting infrastructure.

Open `https://klipa.peterdsp.dev` in a real browser and check desktop and narrow/mobile layouts, language switching, navigation, actual platform download links, release/version information where shown, welcome/privacy/press pages, and meaningful console/network failures. Check both the API-success and API-failure download paths. Verify HTTPS and redirect behavior, no broken primary assets, and correct final download filenames/versions. Compare downloaded checksums with the published release evidence.

Record the production deployment URL, deployment identifier/run, source commit, timestamp, and observed live result. Fix issues discovered during this pass, merge the fixes into main, redeploy, and repeat the affected checks. A local preview, HTTP 200 response, or successful build alone does not establish production completion.

Do not call the release done because a workflow started, an artifact uploaded, or a release page exists. Missing credentials, rejected notarization, a failed physical test, or an unresolved restoration bug is a real remaining gate. Complete unaffected work and provide the exact blocker instead of substituting an unsigned or unverified public build.

## 10. Evidence, completion, and final report

Maintain concise durable records so the work can continue without repeating discovery:

- `docs/keep-awake-investigation.md`: reproductions, demonstrated causes, source-backed mechanism choice, alternatives tested and rejected.
- `docs/keep-awake-verification.md`: automated and physical results, configurations, actual durations, artifact hashes, evidence locations, remaining gaps.
- `docs/keep-awake-release-checklist.md`: exact candidate commit/version, app/helper compatibility, distribution checks, asset URLs/hashes, channel status, rollback procedure.

Include a delivery ledger with separate states for implementation, merge to main, CI, physical verification, signing, notarization, public release, updater, Homebrew/Scoop, winget/AUR, App Store, website deployment, and production smoke checks. Distinguish required work completed from external review pending, access blocked, and untested. Preserve immutable artifact identity when post-release documentation or manifest commits advance main beyond the release tag.

Use existing equivalent files if they already exist. Keep sensitive raw diagnostics out of the repository and public release assets. Keep results separate from planned tests.

Completion requires all of the following:

- The original failing scenario is reproduced or sufficiently characterized and demonstrably fixed on the relevant real hardware.
- Ordinary sleep preferences no longer interrupt an active supported session; closed-lid work continues through the required tested transitions.
- State labels describe effective protection, failures are actionable, and unattended session expiry works.
- Original owned settings restore after stop, expiry, crash, and relevant lifecycle transitions; unresolved restoration never disappears from state or recovery records.
- The privileged component is authenticated, bounded, independently recoverable, and correctly installed/upgraded.
- Clipboard/history/license/menu behavior and supported platform/build variants pass the relevant regression checks.
- The tested, signed, notarized macOS artifacts are actually published under a new version, with functional update/download paths and accurate release claims.
- All required implementation, workflow, documentation, and applicable manifest changes are merged into remote main, and their required checks pass.
- The production website is deployed from the intended source, its live user flows and downloads work, and its claims match the published artifacts.
- Each configured distribution channel has a verified delivered state or a precise external blocker; no pending external approval is presented as public availability.

Finish with a concise factual report: causes fixed; main implementation changes; exact hardware/OS cases and durations passed; remaining limitations or blocked gates; merged PRs and final main commit; new version/tag and release commit; public release/artifact links; signature/notarization/install/update status; package-manager/App Store states; production URL and deployment evidence. Clearly label anything not tested or not publicly available. Do not ask whether I want you to finish a merge, publication, or deployment that this prompt already authorizes.

Start with current-state inspection and access checks, preserve the five commits, reproduce and implement all remaining requirements, verify, merge, package, obtain the physical evidence, publish, deploy, and confirm production. Continue until the requested delivery is complete or a concrete external dependency prevents the remaining step. Complete every independent step before handing back a blocker. Do not replace the engineering work with another plan, an optimistic claim, or a cosmetic toggle change.
