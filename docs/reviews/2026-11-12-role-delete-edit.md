# Role deletion and double-click editing

Commit `6e804c4` adds a visible Delete Role action with confirmation and Cancel first. Saved-role deletion reuses the existing archive mutation, preserves historical records, and removes archived roles from the normal list. Unsaved local drafts can be discarded without a core mutation. Active bindings, saved draft bindings and locally edited bindings are checked before deletion; the store remains authoritative. The last deleted saved role does not immediately regenerate a starter role.

Double-clicking a role selects the Role Settings tab and focuses its name. Asynchronous role reads now carry a selection token, preventing older completions from overwriting a newer selected role or local draft.

Validation: Swift brain/real-control harness **120 passed, 0 failed**. New checks cover double-action wiring, editor navigation, visible deletion action and refusal to delete a bound role. Existing tests cover role references and real control requests. The target Intel machine compiled the host with Swift 5.7.2; app signature strict verification passed.

Installed on disk as `6e804c4-device-20261112`, with old app/config backed up under `$TARGET_HOME/.wechat-rpa/acceptance/6e804c4-role-delete`. The current live host was deliberately not restarted: its actual screen contains an unsaved role draft and a revision_conflict message. Before/after host PID lists are identical, preserving that editing session. New interactions have automated coverage but are not yet verified in the running live UI. No business role was deleted, activated or rebound during installation; config/core state were not changed. The previously diagnosed empty-activation and missing role-status projection issues are outside this change and remain unresolved.
