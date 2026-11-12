# Reply restoration

User reported that replies stopped after role work. Live evidence showed generation 7 with the selected role correctly active as default, WeChat logged in, but admin-pause.json still present with paused=true. Core PID 97880 reported paused/admin_pause and all three configured sessions stopped. This acceptance pause, rather than an inactive role, was the immediate block.

Executed the existing `control resume` command against the active config. No code, allowlist, role bindings or model configuration changed. A contacts sync then occupied the desktop and was interrupted with user_input_detected; normal scheduling subsequently resumed without manually clicking or retrying the failed sync.

At 15:15 local time, all three sessions (测试群, 午夜咖啡, 爱咖啡) reached watching. The 午夜咖啡 turn progressed through generating, sending, verifying_send and watching. Health recorded last_client_echo_observed=true and send ID f7c61c72-1e43-4492-8e8d-beb230f9957b. Delivery remains unknown and platform_message_verified=false; client echo is not recipient delivery confirmation. A passive desktop screenshot also showed the new reply preview in the conversation list. No diagnostic message was injected; this was the normal automatic reply pipeline processing the existing incoming message.

Leave the global pause removed. No restart was necessary. Acceptance pause must not be mistaken for normal operational state in subsequent status reports.
