# 生产更新：安装记录与防降级水位分离

用户已确认契约中的schema 2兼容方案。本阶段实现记录及读取守卫，不把这一前置能力称为完整安装验收。

## 修改

- current_release.json同时支持v1/v2。v1继续以sequence表示水位，序列化不增加字段；v2必须显式包含整数highest_accepted_sequence且不小于当前安装sequence。null、布尔、浮点、未知字段及未知schema拒绝读取。
- v2提交前核对原水位，拒绝降低；拒绝以v1覆盖已存在的v2记录。v2仅原子提交权威记录，不再同步写第二份水位文件，避免回退中的双文件提交窗口。较低的合法旧水位镜像保持可读；高于权威记录或损坏的镜像要求恢复处理。
- Rust和Swift启动守卫同步校验两种记录格式，继续绑定事务、安装版本、摘要和受控current目录；journal schema仍为1。Swift数值读取保留完整UInt64范围，拒绝布尔及浮点截断。
- 只读检查不会迁移记录。回退后，来源恰好匹配已登记的当前安装版本时可显示already_installed；下载仍要求清单序号严格高于历史最高水位。

主要文件为src/updater/journal.rs、src/updater/check.rs和native/bootstrap/CoreSupervisor.swift。其余Rust构造器显式保留v1格式，未提前启用生产写入。

## 验证

- updater定向测试54通过、0失败、1忽略。
- Swift宿主集成78通过，trusted_paths_test 7通过。
- Rust与实际编译执行的Swift守卫共享v2合法/非法格式、UInt64上限与旧镜像冲突场景；补充镜像场景的定向测试通过。
- 更新页面自测首次完整运行发现测试生成的旧镜像权限不符合私有文件约束。修正测试文件为0600后，该目标通过；没有放宽生产读取要求。
- 完整cargo test --locked复跑2365通过、0失败、3忽略，结果在本地target/updater-record-v2-full-retest.log；随后补充的旧镜像共用场景定向测试也通过。格式、差异及敏感信息检查通过。

## 剩余范围

本阶段没有修改专用机稳定入口、业务数据或安装版本。真实基线登记、生产排空/备份/切换/健康验证、程序回退、崩溃恢复以及本机签名包A到B部署验收仍需完成。

Orca此前返回no_active_sender_terminal，当前会话没有可用的派发身份；未冒用其他终端提交完成报告，也不宣布整个项目验收通过。
