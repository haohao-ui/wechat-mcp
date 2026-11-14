# 生产更新：配置发布源的只读检查

在候选准备验证之后，接通现有检查接口与配置发布源。完整安装/回退尚未完成，current_release记录v2方案仍等待确认。

## 实现

- control update-check读取可选updater.source，本机源或HTTPS源共用原有受限传输；也保留显式manifest路径的只读检查。无源仍返回unconfigured。无效配置明确失败，不再悄悄回退到另一个state根。
- wechat-updater check在显式配置发布源时可省略manifest参数；旧manifest/trusted-keys离线验签入口保留，未配置源时仍要求manifest。
- 先验签，再校验声明的系统、架构、完整组件元数据和schema兼容范围。实际系统版本由有时限的sw_vers取得，不再固定假设12.0。同步control从专用线程驱动检查，避免嵌套Tokio runtime。
- 相同sequence不能直接显示已安装：必须有匹配的权威记录、清单摘要、Committed journal和current指针守卫。只有历史水位而缺少安装记录时拒绝该判断。
- TrustedKeyStore补齐原发布格式的有效性检查：仅接受已支持的封套/清单schema 1，拒绝失效区间、已过期清单和超过5分钟时钟容差的将来发布时间。没有新增或修改current_release/journal字段。
- Swift兼容数字Unix发布时间与旧字符串日期；显示“清单签名已验证”“安装包待校验”，不把清单验签冒充完整包验证，也不把匹配记录冒充正在运行。没有发布者验签证明的available/already_installed响应被降为未验证状态。
- 下载、安装、延期和恢复的UI写操作仍未启用，本次没有安装能力放行或模拟成功。

## 实际测试

- updater模块：48通过，0失败，1忽略。新增配置源检查无状态写入、仅水位不得冒充安装、安装记录/journal/current匹配、签名过期/将来/未知格式、同步control在Tokio调用内工作及无效配置失败测试。
- Swift更新UI集成目标通过，包含109项内部断言：数字发布时间、验证范围提示、缺少验签字段不显示验证成功及原只读边界。
- trusted_paths_test：7通过；原control update相关单元测试：5通过。
- Intel release核心/更新器构建成功；fmt、diff、敏感信息及脚本语法检查通过。

## 专用机实际验证

使用独立目录中的新核心与新更新器，读取上一阶段的隔离配置和真实发布者签名包1.0.0-updateprep4.20261114。未替换稳定App或bin。

- 两个CLI的检查结果完全一致，返回available、发布者已验证及数字发布时间。
- 用同一真实发布者密钥签署的已过期测试清单，通过显式只读路径检查后被拒绝；不加入选定发布源。
- 篡改选定源的签名后，检查返回incompatible且candidate为null、publisher_verified为false；随后恢复源文件。
- 比对隔离state中所有文件的路径、大小和内容摘要，检查前后完全一致，没有下载包、写候选缓存、维护标记或安装记录。
- 当前真实配置以及稳定宿主、核心、更新器的摘要和安装版本与上一阶段准备前一致。没有进行安装。

完整私有证据保存在本地target/updater-source-check及专用机隔离验收目录，不提交客户数据或密钥。

## 后续

还需接通GUI下载，以及基线登记、生产事务、真实排空/备份/切换/启动证明、程序回退、崩溃恢复和最终部署验收。记录v2确认前不修改共享记录格式或原启动守卫。项目总验收没有完成。
