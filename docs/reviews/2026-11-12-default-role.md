# Default role setup

The Brain role page offers **添加默认角色** to create an editable, unsaved starter role from the bundled `brain-default-role.json` resource. It includes persona, goals, tone, business rules and fallback replies. The template contains no account, contact or knowledge references; each use creates a new role ID. It does not overwrite existing roles.

After reviewing the content, choose **保存草稿 → 设为默认角色… → 保存绑定草稿 → 启用配置**. The shortcut only changes the default role in the binding draft; contact-specific overrides remain intact. Existing account isolation, generation checks and activation confirmation are reused. Adding a template or choosing a default does not automatically activate it or resume paused replies.

The template resource is copied by the release packaging and host deployment scripts. Business content remains editable in the role form after creation, rather than being compiled into runtime behavior.

Validation: the Swift brain/real-control harness passes 112 checks, including resource content and fallback decoding, fresh IDs without account/knowledge leakage, default-binding shortcut selection, and preservation of the current draft's contact overrides. Both deployment shell scripts pass `bash -n`. The first override test compared against an earlier fixture rather than the current edited draft; it was corrected to capture the pre-action draft and then passed.

The first installed build `4baa1bd-device-20261112` compiled with Swift 5.7.2 on the macOS 12.7.6 Intel device, passed strict signature verification, and loaded the template into the actual role form. The core remained paused/admin_pause. Live long-text inspection exposed a missing text-container width bound; commit `998d85f` adds it and the 112 checks pass again. No role or binding was activated during these checks.

Final live result: `13b8c9d-device-20261112` is installed and signed. On the dedicated machine, selecting Brain and pressing Add Default Role successfully populated the editable role and fallback content. The final screenshot confirms the long persona wraps onto multiple lines. Evidence: `$TARGET_HOME/.wechat-rpa/acceptance/default-role-final/live-template.png`. Final core health: PID 68121, paused/admin_pause. The form is intentionally left as an unsaved starter draft for review; no default binding was persisted or activated. The immediately previous application and config are backed up in the same private acceptance directory. No messages were sent.
