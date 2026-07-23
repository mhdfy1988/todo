# 代办：桌面悬浮待办

“代办”是一款本地优先的 Windows 桌面悬浮待办：想到事情时快速记下，按自己需要的顺序处理，并保留完成历史作为周报和回顾依据。

当前公开版本为 [`0.1.2`](https://github.com/mhdfy1988/todo/releases/tag/v0.1.2)，属于可试用的早期版本，新增一级子代办并把本地账本升级到 schema v5。Windows x64 NSIS 安装器、更新签名与 `latest.json` 由发布工作流闭环验证；`0.1.1 → 0.1.2` 的应用内升级和真实数据迁移仍需在已安装的 `0.1.1` 上完成实机核对。宠物、番茄钟、AI 和日历均不在本阶段范围内。

## 功能

- 快速记录待办，并在展开面板、当前任务胶囊和系统托盘之间切换。
- 每条待办都可直接勾选完成，也可软删除；删除会移出待办队列，但保留事件历史且不产生奖励。
- 双击未完成待办标题，或聚焦后按 `Enter` / `F2`，可在原位置修改标题；`Enter` 或失焦保存，`Esc` 取消。
- 截止日期默认不设置，可在编辑待办时额外选择或清除。列表只在有期限时显示“今天”“明天”“M/D”“YYYY/M/D”或“逾期 N 天”等紧凑标签。
- 通过拖动柄或 `Alt+↑/↓` 调整完整待办顺序。
- 在待办页和完成记录页按 `Ctrl+F`，或从更多菜单进入当前页面搜索。搜索只按标题实时筛选并高亮；搜索期间仍可完成、修改、删除或撤销，但禁止重排过滤后的局部列表。
- 独立查看完成记录和完成时间，并通过 `↶` 撤销完成；撤销后的任务回到待办队尾。
- 在完成记录标题右侧点击“复制本周”，可把本地自然周内仍然有效的完成项复制为 Markdown 编号列表；本周为空时不会覆盖剪贴板。完整周报、上周或自定义日期仍属于后续能力。
- 长列表只在列表内部滚动，录入区、完成记录标题和底部入口保持固定。
- 正常模式启动后会检查更新，并每 24 小时再次检查；也可从更多菜单手动检查。发现新版本后，由用户明确点击安装。

### 0.1.2：一级子代办

- 顶层代办可以渐进展开一级子代办。没有子代办时不显示 `0/0` 或常驻工具栏；悬停或键盘聚焦父项时才出现添加入口，有子代办后以紧凑进度显示完成情况。
- 展开后从底部“添加子代办”进入单次添加态：非空标题按 `Enter` 或移开焦点创建，成功后退出并恢复添加入口；空白失焦直接退出，不创建也不报错。子标题支持原位修改，按 `Enter` 或失焦保存；子项还可软删除、完成、撤销完成，并在同一父项内拖动或使用 `Alt+↑/↓` 重排。
- 勾选父项会在一次可靠写入中自动完成全部未完成子项，再完成父项；子项各自保留完成事实但不发金币，父项仍只奖励 `1` 枚金币。单独完成最后一条子项不会自动完成父项，仍需用户明确勾选父项。
- 搜索会保留父项上下文和整组真实进度；完成记录按父项分组并默认折叠；胶囊显示“父项 / 当前子代办”以及父项期限。父项在本周最终完成时，周回顾会把更早完成且仍有效的子项作为缩进明细；普通子项只有在本周完成时才单独进入本周结果。
- 第一版只支持一级；子代办没有独立期限和金币，不能跨父项排序，也不能在父项与子项之间互相转换。

## 技术架构

- 桌面壳：Tauri 2。
- 前端：原生 HTML、CSS、JavaScript 模块（ES modules），不依赖前端框架和构建器。
- 核心与应用服务：Rust，负责任务状态、队列、事件、奖励、幂等和窗口运行模式。
- 持久化：SQLite，本地任务投影、事件、奖励和命令回执在同一事务中提交。
- 可靠写入：前端通过操作箱保存稳定操作 ID，再经 `LedgerSession → TauriGateway → Tauri 命令 → TaskService` 提交意图；前端只渲染后端快照，不复制领域状态机。
- 子代办交互：`SubtaskController` 只维护展开、单次添加态、子项编辑、组内重排和焦点等临时状态；`snapshot.subtasks` 与 `snapshot.effectiveCompletions` 是后端投影，新增子项和同组重排分别使用 `create_subtask`、`reorder_subtasks`，其他操作复用现有可靠写入链路。
- 父项完成：前端只提交一次 `complete_task` 意图；Rust 在同一 SQLite 事务中完成仍为 `pending` 的活动子项，再写父项主完成事件、`+1` 奖励和唯一外部命令回执。已完成子项不重复写，已软删除子项不参与级联。
- 本周完成复制：`WeeklyCompletionController` 复用现有只读 `weekly_facts` 查询，通过项目自己的剪贴板写入端口调用 Tauri 官方插件；应用只申请文本写入权限，不读取剪贴板。
- 更新适配：`UpdateController → TauriGateway → 项目自有更新命令 → AppUpdateService → Tauri updater`。前端只负责检查时机和交互，Rust 负责运行模式校验、待安装版本与单次安装锁。

代码依赖方向和可靠性不变式见[《代码架构与模块边界 v0.1》](./docs/代码架构与模块边界-v0.1.md)，事件与任务状态说明见[《任务状态与事件账本 v0.1》](./docs/任务状态与事件账本-v0.1.md)，一级子代办的交互与领域边界见[《子代办完整交互与领域设计 v0.1》](./docs/子代办完整交互与领域设计-v0.1.md)，发布边界与操作流程见[《Windows 发布与自动更新 v0.1》](./docs/Windows发布与自动更新-v0.1.md)，面向使用者的版本变化统一记录在 [`CHANGELOG.md`](./CHANGELOG.md)。

## 运行与构建

### Windows 前置条件

- Windows 10 或 Windows 11。
- Node.js 与 npm，建议使用当前维护中的 LTS 版本。
- 通过 `rustup` 安装的 Rust stable 工具链。
- Visual Studio 2022 Build Tools，并安装“使用 C++ 的桌面开发”工作负载、MSVC 工具集和 Windows SDK。
- Microsoft Edge WebView2 Runtime。

以下命令均从仓库根目录运行。Windows PowerShell 中使用 `npm.cmd`，避免执行策略拦截 `npm.ps1`。

安装锁定依赖：

```powershell
npm.cmd ci
```

启动开发版：

```powershell
npm.cmd run desktop:dev
```

运行统一测试门禁：

```powershell
npm.cmd run desktop:check
```

该命令会先运行前端 Node 测试，再运行 Rust 测试。需要进一步验证本地账本或桌面联合链路时，可运行：

```powershell
npm.cmd run ledger:smoke
npm.cmd run desktop:smoke
```

构建不带安装器的 release EXE：

```powershell
npm.cmd run desktop:build
```

产物位于 `src-tauri/target/release/zuoban-desktop-spike.exe`。用户可见产品名与 Windows 文件元数据为“代办”；内部 identifier、EXE 名和数据库文件名继续保留 `zuoban` 兼容名称，避免破坏已有本地数据。

构建 Windows x64 NSIS 安装器与更新签名：

```powershell
npm.cmd run desktop:bundle:windows
```

安装器与 `.sig` 更新签名位于 `src-tauri/target/release/bundle/nsis/`。本机已经验证两类产物可以成功生成；[`v0.1.2` GitHub Release](https://github.com/mhdfy1988/todo/releases/tag/v0.1.2) 的线上 `latest.json`、更新签名和资产摘要由发布工作流验证，完整版本间升级闭环仍待已安装的 `0.1.1` 实测。

正式发布前需要先试装时，使用独立候选配置生成“代办测试版”：

```powershell
npx.cmd tauri build --bundles nsis --ci --no-sign --config src-tauri\tauri.candidate.conf.json
```

候选配置使用独立 identifier 和应用数据目录，不覆盖正式“代办”；同时关闭 updater 产物和 Authenticode 签名，只用于本机功能试用，不能作为正式 Release 资产。

## 安装、发布与更新

- 首次安装：用户从 [`v0.1.2` GitHub Release](https://github.com/mhdfy1988/todo/releases/tag/v0.1.2) 手动下载 Windows x64 `setup.exe` 并安装；安装范围为当前用户。
- 后续更新：正常模式启动后立即静默检查一次，此后每 24 小时检查；更多菜单提供“检查更新”。发现新版本后，用户点击“安装更新”才会下载、安装并重启应用。
- 冒烟隔离：smoke 前端不会启动更新控制器，Rust 更新服务也会在访问网络前拒绝请求，避免测试触碰正式更新源。
- 自动发布：`.github/workflows/release.yml` 支持从 `main` 手动触发和 `v*` 标签触发；已公开的版本禁止覆盖，新版本必须先同步提升三个版本号，并把 [`CHANGELOG.md`](./CHANGELOG.md) 的 `Unreleased` 内容收口到带实际日期的当前版本段。GitHub Release 正文只读取该标准版本段，不混入自动提交列表；`tauri-apps/tauri-action@v1` 先生成草稿 Release、安装器、更新签名和 `latest.json`，工作流回读核对正文、用客户端公钥验签并核对元数据后才自动发布。
- 发布密钥：仓库只约定 GitHub Actions Secrets `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；私钥和密码不得提交到仓库，应在受控的加密离线位置保留恢复备份。
- Windows Authenticode 代码签名暂未配置。Tauri 的 `.sig` 用于校验自动更新产物，不能替代 Authenticode；首次运行安装器时可能出现 Microsoft Defender SmartScreen 提示。

## 数据与隐私

- 待办、完成历史、事件和奖励账本保存在本机 Tauri 应用数据目录的 SQLite 数据库中。
- 核心待办流程可离线运行，不接入云同步、AI 服务或外部日历，也不会把截止日期发送到外部系统；只有正常模式的更新检查会访问 GitHub Releases。
- 写命令使用幂等回执和可靠操作箱；结果未知时会保留原操作并锁住后续写入，不会静默重复执行。
- `0.1.1` 使用数据库 v4；`0.1.2` 的一级子代办把本地结构升级为 v5，并保留 v4→v5 迁移前备份、旧数据和旧回执兼容。降级安装程序不会自动降级数据库。
- 面向用户的删除是可追溯软删除，不会物理清除历史。永久清理、通用自动备份、手动导出和跨设备恢复尚未实现。

## 已知限制

- 当前只按 Windows 桌面环境开发和验证，不承诺 macOS 或 Linux 可用。
- 当前安装器只面向 Windows x64；`v0.1.2` 的公开下载、元数据和更新签名由发布工作流验证，`v0.1.1 → v0.1.2` 应用内更新与 schema v4→v5 真实数据迁移仍待实机验证。
- Windows Authenticode 暂未配置，安装器可能触发 SmartScreen 提示。
- 多显示器、所有 Windows 缩放比例以及任务栏位于四个方向的组合仍需更多实机验证。
- 截止日期只用于本地展示，不会自动排序、隐藏任务、提醒、发送通知、同步日历或产生奖惩。
- `0.1.2` 的子代办只支持一级，不提供子项期限、子项金币、跨父项移动、父子转换、批量粘贴或 AI 自动拆解。
- 当前不提供有道待办同步或历史迁移；产品仍处于试用阶段，尚未达到完整替代有道待办的发布标准。
- 宠物成长、番茄钟、AI、日历联动、完整周报、阻塞/恢复工作流、通用备份和导出均为后续方向；当前只提供固定自然周的有效完成项 Markdown 复制。

## 设计与文档

极简原生风是当前实现和验收基线。展开面板只保留快速记录、统一待办列表和完成记录入口；低频技术操作收进右上角更多菜单。

### 当前主方案

- [极简原生主方案完整流程原型](./prototype/native/index.html)
- [桌面悬浮窗口技术实验 v0.1](./docs/桌面悬浮窗口技术实验-v0.1.md)
- [本地事件历史、金币账本与异常恢复技术实验 v0.1](./docs/本地事件历史、金币账本与异常恢复技术实验-v0.1.md)
- [界面与交互设计说明 v0.2](./docs/界面与交互设计说明-v0.2.md)
- [产品设计方案 v0.1](./docs/产品设计方案-v0.1.md)
- [产品路线图 v0.1](./docs/产品路线图-v0.1.md)
- [子代办完整交互与领域设计 v0.1](./docs/子代办完整交互与领域设计-v0.1.md)
- [一级子代办交互原型](./docs/prototypes/subtasks-v0.1/index.html)
- [Windows 发布与自动更新 v0.1](./docs/Windows发布与自动更新-v0.1.md)
- [标准更新日志](./CHANGELOG.md)

### 保留候选设计

三种视觉设计全部保留，但第一版不并行开发三套完整主题：

- [极简原生风格研究稿](./prototype/themes/native.html)——当前主方案。
- [深色透明风](./prototype/themes/glass.html)——后续候选皮肤。
- [像素桌宠风](./prototype/themes/pixel.html)——后续候选皮肤。
- [三种风格统一比较入口](./prototype/themes/index.html)
- [视觉方向决策 v0.3](./docs/视觉方向决策-v0.3.md)
- [三种视觉风格对比 v0.2](./docs/视觉风格对比-v0.2.md)

深色透明和像素桌宠只复用同一套组件、协议和业务逻辑，不会各自复制一套任务实现。

### 历史探索

- [第一轮暖色手账风原型](./prototype/index.html)
- [第一轮界面设计说明 v0.1](./docs/界面设计说明-v0.1.md)

暖色手账风只作为历史探索保留，不是当前实现基线。

### 浏览器预览原型

主方案网页原型用于保存完整业务设想，不能替代真实 Tauri 桌面应用。默认可直接打开 `prototype/native/index.html`；若浏览器限制本地脚本，可在仓库根目录运行：

```powershell
python -m http.server 4173 --bind 127.0.0.1 --directory .\prototype
```

然后访问 `http://127.0.0.1:4173/native/`。

## 许可证

本项目使用 [MIT License](./LICENSE) 开源。
