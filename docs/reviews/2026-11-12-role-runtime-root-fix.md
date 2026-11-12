# Role application in live reply runtime

Live diagnosis confirmed that the active role was role_6E2E9A8C/rev_1, named 新助手, with the persona 您是一位专业友善的客服助手。 Recent per-session send journals had no fixed_snapshot/role provenance, and the shared history database had no brain_turns table. The observed generic introduction was consistent with BuiltinBrain's legacy fallback prompt, not evidence that the activated role was used.

Root cause: BotCoordinator assigns each child's config.state_dir to its sessions/contact directory. Turn execution looked for brain-bindings.json in that child directory, found no store, and silently chose Legacy. The fix uses the account root identified by the shared HistoryStore, retaining session-local cursors/journals and account-scoped profile access checks.

A second upgrade defect was reproduced: the device had schema migration 5 recorded but lacked brain_turns. Added an idempotent migration 6 that creates the already-defined role provenance table/index when absent. This leaves existing applied migrations and business data intact.

Regression evidence: the role-provenance turn test now uses the actual child-session directory layout; it failed before the path fix and passed afterward. A legacy-version-5-without-table test also failed before migration 6 and passed afterward, including repeat migration. Final full library run: 1007 passed, zero failed, one ignored. Intel release build passed. Source formatting followed validation without semantic changes.

Installed signed core from commit a0c4d11 while preserving the running host/UI and current config. Before replacement, core stopped gracefully and the account state was archived and tar-verified. Backup directory: `$TARGET_HOME/.wechat-rpa/acceptance/role-runtime-fix`; state archive SHA256 b02994bac8bc61f315fad46c2178986b748adda4de6cdff3fcafeb3459429727. Candidate and installed signed binaries compare identical. Live startup applied migration 6 and brain_turns is present. New core PID 2532. Global pause was removed after checking migration; coordinator returned to watching.

The user's saved role name is still 新助手; no role content or bindings were changed. A fresh inbound message from 示例联系人甲 was requested to verify a new live role snapshot. At the first post-install query there were no new brain_turns rows, so live role-conditioned output remains pending that incoming message. Do not equate unit success, schema creation or watching state with confirmed role-conditioned customer reply.
