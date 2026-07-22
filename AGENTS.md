# 代办项目规则

## 架构边界

1. 任务状态、队列、事件、金币和幂等规则只在 `src-tauri/src/ledger/domain.rs` 与 Rust 应用服务中实现；前端只提交意图并渲染后端快照。
2. `desktop/app.js` 与 `src-tauri/src/app.rs` 是两个组合根。功能模块通过显式依赖接入，不自行读取散落的全局服务。
3. 页面、窗口回调和视图组件不得直接调用账本写命令；统一经过 `LedgerSession → TauriGateway → Tauri 薄命令 → TaskService`。
4. 核心层依赖项目自己的端口；Tauri、SQLite、localStorage 等通用能力只出现在适配层。新增第三方库时也必须经适配层隔离。
5. `integration_smoke.rs` 可以单向依赖 `runtime_profile.rs` 和 `frontend_probe.rs`；生产运行配置和前端探针不得反向依赖冒烟实现。

## 可靠性不变式

1. IPC 命令名、camelCase payload、稳定错误码和操作箱 v1 key 属于兼容协议；变更必须同步网关、Rust 命令、契约测试和技术文档。
2. 写命令跨 IPC 前必须先保存稳定 `operationId`。结果未知时保留同一 ID 并锁住后续写入；只有明确的确定性领域拒绝可以清理未提交操作。
3. 持久化操作箱读取时必须按命令校验 payload；记录损坏时保留原文、显式报错并锁住写入，不得自动删除。操作箱持久化清理成功后，才能清除内存 pending 并恢复交互。
4. 命令确认后仍需刷新真实快照，成功后才能清理操作箱；较早请求的迟到快照不得覆盖较新快照。
5. `READY` 只能在内部运行锁释放后发布；按钮启用、`canMutate()` 与前端就绪报告必须使用同一交互口径。
6. 一次领域变化只能通过一个 `SqliteLedgerStore::commit_transition` 提交；任务投影、事件、奖励和命令回执必须同事务成功或回滚，禁止拆成公开的分步写入。
7. 禁止把前端内存数组、旧窗口链路或其他旧实现保留为静默回退。
8. normal 与 smoke 必须在窗口状态文件、数据库和操作箱三层隔离；冒烟不得读取、写入或清理正式数据及恢复凭据。
9. 托盘或系统回调改变窗口模式时必须统一经过 `window::set_mode`，并用 `window-status-changed` 把完整 `WindowStatus` 同步给前端；不得只改原生窗口尺寸而让 `body.dataset.mode` 保留旧值。隐藏到托盘本身不得暗中切换显示模式。
10. 面向用户的“删除待办”默认是可追溯软删除：写入 `Abandoned` 事件并从队列移出，不物理删除任务或历史、不产生奖励；真正的永久清理必须另行设计和确认。
11. 标题修改只允许立即 `pending` 待办，必须通过 `update_task_title` 追加 `TitleUpdated`（存储值 `title_updated`）事件并保存 `beforeTitle` / `afterTitle`；不得直接覆盖旧事件，也不得改变任务状态、队列位置或金币。
12. 应用更新只能由前端 `UpdateController` 经 `TauriGateway` 调用项目自有 Rust 命令，再由 `app_update` 适配 Tauri updater；自动检查只能在 normal 账本进入 `READY` 后运行，smoke 必须在前后端两层禁网，安装必须由用户确认且不得与未完成账本写入并发。
13. `plugins.updater.pubkey` 是已安装客户端的更新信任身份；发布后不得随意替换。更新私钥和密码只允许进入本机安全备份与 GitHub Actions Secrets，禁止提交仓库；私钥丢失时不得以无签名或新密钥静默绕过既有客户端校验。
14. GitHub Release 必须先以草稿接收安装器、`.sig` 与 `latest.json`，再用客户端内嵌公钥验证实际签名并核对元数据；只有验证通过才能自动转为正式 Release，禁止未经闭环校验直接公开更新产物。
15. 已公开的版本、安装器、更新签名与 `latest.json` 视为不可变发布产物；工作流必须拒绝覆盖同版本正式 Release。失败后的未公开草稿可以同版本重试，正式发布后的修复必须同步提升 `package.json`、Tauri 与 Cargo 三处版本号并创建新 Release。

## 模块化约束

1. 前端继续使用原生 ES 模块、JSDoc 与 Node 内置测试。只有出现当前结构无法解决的明确复杂度后，才评估前端框架、构建器或状态库。
2. 只为真实重复、变化点或外部边界增加模式；不要引入依赖注入容器、通用 `BaseRepository`、全局事件总线或按表 CRUD 抽象。
3. 视图和选择器保持只读；状态迁移集中在 `LedgerSession`、`TaskService` 和 Rust 领域转换。
4. 窗口尺寸、焦点与贴边策略统一由 `WindowMode::spec()` / `WindowSpec` 提供，不得在命令、托盘和前端分别复制常量。
5. 新增业务能力前先复用现有契约、应用服务、端口与适配器；需要新增边界时，同步更新 `docs/代码架构与模块边界-v0.1.md`。
6. 透明原生窗口中，占满窗口的展开面板与胶囊表面不得使用会进入圆角外侧的阴影；应保留克制的轮廓边框，避免面板在白色桌面上失去边界。相关样式变更至少在浅色背景和当前 Windows 缩放比例下同时核对外侧透明度与窗口可辨识度。

## 验证基线

默认从仓库根目录运行统一门禁 `npm.cmd run desktop:check`，它会依次覆盖前端和 Rust 测试。修改 IPC、恢复、事务或运行模式隔离时，还必须运行对应的账本冒烟和桌面联合冒烟。

本地浏览器的物理拖动自动化可能只产生 `pointerdown/mousedown`，不能据此判定桌面 HTML5 拖放失效。排查任务重排时先确认排序柄 `draggable` 状态与 Tauri `dragDropEnabled: false`，再用事件链回归或真实 Tauri 窗口验证 `dragstart → dragover → drop`。

桌面图标以 `src-tauri/icons/source.svg` 为唯一可编辑源；修改后统一运行 `npx.cmd tauri icon src-tauri\icons\source.svg` 生成 PNG / ICO，不直接手改派生图标。托盘复用正式构建内嵌的 `default_window_icon`，因此验证时必须重新构建、彻底退出旧进程并只启动一个新进程；默认先排查旧构建和旧托盘进程，不以重启 Explorer 作为首选。
