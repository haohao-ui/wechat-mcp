# Default role setup

The Brain role page offers **添加默认角色** to create an editable, unsaved starter role from the bundled `brain-default-role.json` resource. It includes persona, goals, tone, business rules and fallback replies. The template contains no account, contact or knowledge references; each use creates a new role ID. It does not overwrite existing roles.

After reviewing the content, choose **保存草稿 → 设为默认角色… → 保存绑定草稿 → 启用配置**. The shortcut only changes the default role in the binding draft; contact-specific overrides remain intact. Existing account isolation, generation checks and activation confirmation are reused. Adding a template or choosing a default does not automatically activate it or resume paused replies.

The template resource is copied by the release packaging and host deployment scripts. Business content remains editable in the role form after creation, rather than being compiled into runtime behavior.
