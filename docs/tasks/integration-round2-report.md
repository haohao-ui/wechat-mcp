# 集成第二轮（2026-11-17）

本轮仅解决 T8 与集成分支的冲突、只读诊断 134、验证与重编。未执行 commit、merge、rebase、reset、安装、发布、截图或真实微信连接。

## 冲突处理

- `src/core/control.rs`：保留 T2 的 brain-history/session 只读请求校验和固定错误信息；加入 T8 handoff 模块、session-handoff 路由；帮助信息合并保留 model-probe。
- `src/core/control_readonly.rs`：保留 local_date、sent_today、mcp-info busy、brain-history 和全部 T2 测试；保留 T8 的 session_id、reply_mode 与接管/重启/隔离测试。
- 原有 T8 暂存文件原样保留。冲突标记已全部清除；未写 Git 索引，故 `git status` 中 UU/AA 仍在，调度方需要审阅并 git add。

## 134 只读诊断

报告 `wechat-cs-host-2026-11-17-125142.ips` 是本轮检查到的唯一同名前缀报告。

- 异常：EXC_CRASH / SIGABRT，SIGNAL code 6，`abort() called`；shell 134 与此一致。
- 触发线程：0，com.apple.main-thread。
- 调用链：main +392 → NSApplication.sharedApplication → NSApplication.init → _NSInitializeAppContext → NSMenuBarPresentationInstance → GetCurrentProcess → _RegisterApplication → ___RegisterApplication_block_invoke +12752 → abort → pthread_kill。
- 崩溃发生在 AppKit 应用注册阶段，早于 applicationDidFinishLaunching；没有 Swift fatalError、强制解包、数组越界或资源加载失败帧，没有 TCC 权限缺描述的终止说明，也没有 dyld 缺库说明。不能据此保证后续业务初始化没有其他问题。
- 原 `dist/issue-2-app/Apps/微信客服助手.app` 的 Mach-O 是 arm64，Info.plist 语法有效，CFBundleExecutable 匹配；Resources 有 brain-default-role.json、gateway.test.json、AppIcon.icns，与打包脚本要求一致。未读取或复制主人的配置。
- `codesign -dv --verbose=4`：adhoc, linker-signed；Info.plist=not bound；Sealed Resources=none；无 TeamIdentifier。
- `codesign --verify --deep --strict --verbose=2` 失败：code has no resources but signature indicates they must be present。这是已确认的包签名缺陷，但报告不能证明它是本次 SIGABRT 的原因。
- `xattr -lr` 无输出，无隔离属性。
- 系统统一日志读取被沙箱拒绝：`log: Cannot run while sandboxed`。现有验收记录还记载显示抓取失败；怀疑执行环境缺少可用的图形应用注册服务，但未确认。

结论：134 根因 unknown，保留系统注册环境与签名两条线索交产品经理。未重启旧包或新包，未改启动源码、打包签名策略或添加假设性的回归断言。需在授权的正常桌面会话采集系统注册日志，才能决定最小修复。

## 检查与产物

（最终结果见下方追加。）

- `cargo fmt --all -- --check` 与 `git diff --check`：通过。
- 默认 Cargo 配置的 target 指向 HOME 缓存，因 `.cargo-build-lock` 不可写而失败；后续统一 `CARGO_TARGET_DIR=target`。
- `cargo clippy --all-targets --locked -- -D warnings`：失败，lib 报 1938 项 lint；例如未改的 contacts/sync.rs:963 unnested_or_patterns（已核对 HEAD 也存在）。未大规模改动任务范围外 lint，不能声称所有 lint 都已基线对照。
- 按仓库 CI 的 `cargo clippy --all-targets --locked`：退出码 0，带大量警告。
- 默认 HOME 的 `cargo test --locked`：lib 1040 通过、2 失败、1 忽略。两项 gateway 失败中一项明确为资源锁 Operation not permitted，另一项预期错误文本不匹配；全新临时 HOME 下 gateway 全部 18 项通过。
- 全新临时 HOME 的完整 `cargo test --locked`：lib 1041 通过、1 失败、1 忽略。失败为 model_probe 本地测试服务器 read 返回 WouldBlock；该项单独复测通过（1/1），保留首次失败证据，不宣称全套通过。
- 隔离 HOME 下补跑 `cargo test --locked --bins --test '*'`：主程序 1046 通过、1 忽略；后续 swift_action 24 通过、5 失败，主要为 target_not_reached/退出码不符；其后集成测试因 Cargo 提前停止未全跑完。未改动这些桌面动作测试或继续操作桌面排查。
- 专项 `cargo test --locked --test session_handoff_cli --test swift_session_handoff --test swift_host`：1 + 1 + 81 全部通过。T2 readonly 模块 8 项（含 T8 接管测试）、turns 的生成期间接管测试均通过。
- `swiftc -target "$(uname -m)-apple-macos12" native/bootstrap/*.swift native/eyes/Screens.swift -o /tmp/issue2-host`：通过，额外指定临时 module cache 以适配沙箱。
- `scripts/build_release_package.sh 0.2.0-test dist/issue-2-round2-app 1002 test false`：退出码 0；设置工作区 Cargo target 和临时 Swift/Clang cache；产物为 arm64，未生成发布签名。
- 新包 Info.plist 有效、可执行文件 arm64、无 xattr；新包仍只有链接器临时签名，严格 codesign 验证仍失败（同旧包）。本轮未修未确认与崩溃直接相关的签名策略。

新应用：`dist/issue-2-round2-app/Apps/微信客服助手.app`。
归档：`dist/issue-2-round2-app/wechat-mcp-0.2.0-test-macos-arm64.tar.gz`。
SHA256：`ef729b610eb166396ea25dcad7a8d82a81ba83b2f8c934867d46ee0e229affb4`。
证据：`dist/checks/round2/`。这些 dist 文件被忽略，不代表已提交或已交付 PR 附件。

Orca Orchestration 按技能尝试 worker_done 报告失败：`no_active_sender_terminal`，缺少 ORCA_TERMINAL_HANDLE；未冒用其他终端身份。另一次只读 run-current 返回 runtime_unavailable。需调度方接收本报告。没有创建 commit，没有宣布整个项目或真实应用 AC 验收通过。本轮没有扩展实施主人决定列表中其他任务。
