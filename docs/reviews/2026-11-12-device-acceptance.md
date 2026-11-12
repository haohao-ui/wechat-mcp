# Device acceptance — preflight

Date: 2026-11-12. Candidate source: `a76ede7`. Status: **partial — candidate not installed or exercised**.

The user requested real-machine acceptance. Checks below were read-only; no deployment, restart, message sending, model requests, database writes, keychain unlock, or update transaction was performed.

## Observed devices

- Local development machine: macOS 27.3, arm64. WeChat 4.1.15 is installed and its signature verifies; neither WeChat nor the customer-service host was running. Orca Accessibility and Screen Recording permissions are granted; these do not prove the target application's permissions.
- Dedicated machine from repository deployment records: `work@192.0.2.10`, reachable over SSH, macOS 12.7.6, x86_64.
- Installed host build: `be0ef5f-20261110123505`. Its installed application passes `codesign --verify --deep --strict`.
- Installed core SHA-256: `dea93b323c5d383ff3ec4f159a4a0e1a7762e3c196121aa7ecc1b5bd1bb05ed8`.
- Host PID 657 has core PID 660 running `serve-core --config $TARGET_HOME/.WeChatRPABot/config.json` and MCP process PID 675. These are different roles, not evidence of duplicate schedulers.
- Configuration schema 1; three allowlist entries. `install_root` is absent. No model credentials or conversation contents were collected.
- Configured state root is `$TARGET_HOME/Library/Application Support/WeChatRPABot/state`. Health PID 660 reports `watching`; health file age was 0 seconds at observation.
- Configured state root contains `core.lock`; no maintenance marker, update journal, or authoritative current-release record was found. Legacy `.wechat-rpa/core.maintenance` was absent.

## Pending acceptance

The installed build differs from the candidate and architecture differs from the locally generated arm64 test package. No new-version real-machine result can be claimed from this preflight.

Confirm the acceptance target before installing an x86_64 candidate. Candidate installation must preserve existing configuration/data/signing identity and prevent automatic customer-message processing during controlled tests. Actual send tests need named test contacts and explicit message scope. Production update/drain/recovery are still incomplete and cannot pass acceptance merely through a manual installation.

Required evidence after a controlled candidate installation: exact executable/build identity; startup guard and shared-path behavior; application UI; local TCC operation; safe test-contact scenarios; rollback/data-preservation evidence. Seven-day continuous operation remains a separate time-based gate.

## Authorized dedicated-machine installation and tests

The user confirmed `192.0.2.10` as the acceptance target. Candidate preparation exposed two real old-SDK compilation defects, fixed locally before installation:

- `66449f5`: annotate ScreenCaptureKit's macOS 12.3 availability, guard its entry point, and compile the macOS 14 SDK-only shadow option only with Swift 5.9 or newer.
- `f08dbcc`: split Vision observation conversion into explicit steps; Swift 5.7.2 had exhausted its type-checking budget on the original expression. Bounding-box conversion and sorting behavior are retained.

Final installed source revision: **`f08dbcc`**, host bundle build **`f08dbcc-device-20261112`**. The staging directory retains its original `a76ede7-stage` name; the source revision and signed-component manifest are recorded separately to avoid claiming that directory name as version evidence.

Observed results:

| Check | Result |
| --- | --- |
| Intel Rust release build, macOS deployment target 12.0 | PASS, core and updater built |
| Target Swift 5.7.2 host and helper compilation | PASS after the two compatibility fixes |
| Native runtime self-test on current SDK and target SDK | PASS, reports desktop_accessed=false |
| Candidate parsing the real configuration | PASS, `config-check` returns ok=true |
| Maintenance marker startup test on device, isolated state/config | PASS, exit 1, exclusive core lock reacquirable, no health/events created |
| Corrupt journal startup test on device, isolated state/config | PASS, exit 1, exclusive core lock reacquirable, no health/events created |
| Staged native/Rust component signing | PASS, same existing certificate as installed host/core |
| Installed host signature | PASS, identifier local.wechat-mcp.host and certificate leaf 3b11063a3070edca9b73469502d85d44307eee54 retained |
| Installed components versus staged signed hashes | PASS, all 10 matched |
| Native diagnostics after installation | Accessibility=true, Screen Recording=true; WeChat missing/ambiguous |
| New host/core real startup | PASS, host PID 53095, core PID 53262 at observation |
| Health | paused, reason wechat_not_logged_in |
| Original configuration compared to pre-install copy | Identical |
| Update status | architecture x86_64; installed release remains unknown; no fabricated release record |
| App UI and real WeChat observations | BLOCKED by locked session and WeChat not logged in |
| Sending, real update transaction and recovery | NOT RUN |

### Controlled switch and recovery assets

Automatic processing was paused using the existing control command, then core was normally stopped. The old host was quit and its LaunchAgent unloaded. An exclusive core lock was acquired before the state snapshot. The snapshot archive was listed successfully before installation.

Recovery directory on target: `$TARGET_HOME/.wechat-rpa/acceptance/f08dbcc-device`, mode 0700. It contains original configuration, LaunchAgent, enable marker, version file when present, old binaries, old app copies, source revision and signed staging hashes. The state archive is 138,640,809 bytes with SHA-256 `5e16bd9b4f6f82644a44cd9a737351df23212e98b8c6d3eea7b898317d3b5376`.

The host LaunchAgent was re-enabled and the new host bootstrapped. Core was initially kept disabled for signature/component checks, then explicitly enabled only after verifying `admin-pause.json` had paused=true. It remains administratively paused pending controlled test-account instructions. No customer message was sent by the acceptance procedure. No rollback was executed; the existence of a backup is not evidence of successful recovery.

### Remaining limitations

The ScreenCaptureKit streaming operation explicitly requires macOS 14; this macOS 12.7.6 target cannot pass that capability, even though diagnostics and compilation now work. Existing worker capture capabilities require separate live acceptance and are not proven by the runtime self-test.

The target screen-sharing session was observed as locked. The user was asked to unlock the dedicated machine and log into WeChat. UI, actual capture/OCR, configured-brain flows and named test-contact messaging remain pending. Production update apply/rollback/recover and real drain remain incomplete; this manual controlled installation does not validate those features or the seven-day operation requirement.

## Login and read-only UI checks

After the user confirmed login, native diagnostics observed WeChat 4.1.15 running with an on-screen window, Accessibility and Screen Recording authorization true. Core health changed to paused/admin_pause. The screen-sharing title still said locked, but the actual captured desktop was unlocked; the title was stale and is not acceptance evidence.

The real update page was visually inspected: x86_64 runtime, unknown installed release, no configured update source/publisher root, and disabled download/install/defer controls were displayed. No role draft was saved or activated by the acceptance operator.

A live defect was found: brain-list returned ready with zero binding candidates, although the read-only history query counted 45 contacts across two accounts. No contacts belonged to the configured device alias. BrainAccess already resolves the recorded logged-in account, but brain control's account context and candidate enumeration still used the device alias.

The fix reuses effective_account_id for brain control and returns the server-selected account_id in brain-list; the UI displays that authoritative value (or unknown for older responses). Existing device-alias roles are not migrated across account boundaries. The existing projection regression was extended to switch to a second logged-in account with a same-name contact and assert its authorized candidate and no leakage of the prior account's roles.

Validation: the new switch assertion failed first (exit 101, zero candidates instead of one); after the fix the projection test passed, 145 brain-related Rust tests passed, and the Swift brain UI harness passed with additional authoritative-account/legacy-unknown parsing assertions. The revised core and host are staged for device testing, not yet installed: a restart could discard an unsaved role draft, so confirmation was requested.

During read-only acceptance the live configuration changed in two allowlist recipient values (entry count remains three, state root unchanged). No configuration-save command was issued by the operator. The writer is not yet established; the change is preserved and the user was asked whether they are editing concurrently. Do not overwrite it with the pre-install backup.

Named test-contact scope and permission to restart after saving drafts are pending. No send operation or automatic-reply resume has been performed.

The `38de5a5` Intel core and updated host compiled successfully. Executing the staged core's read-only brain-list on the dedicated machine returned account_id different from the configured device alias, ready status and **3 authorized binding candidates** (previously 0). This verifies the live lookup fix without replacing the running host/core or activating roles. Installation of this follow-up remains pending protection of the user's unsaved draft.
