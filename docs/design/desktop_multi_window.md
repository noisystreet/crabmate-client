# Desktop 多窗口多连接

> **范围**：`desktop-tauri` 壳的多窗口协调面 —— 开窗、注册表、切换焦点、审批通知、白名单、
> Bearer 分槽。**不改** Server 契约（SSE / 审批 / 错误码 / 工作区语义），**不覆盖**
> Android / Web（它们只有一条连接）。
>
> TUI 的同类能力是**窗口内 lane 列表**（[`tui_multi_server.md`](./tui_multi_server.md) §1–§8）；
> 两端形状不同、问题相同。本文件只讲 Desktop 这一侧。

## 1. 结论先行

- 一个连接 = 一个 WebView 窗口，各自连一台 `serve`，各自一份 JS 堆。
- 窗口**互相完全遮挡**（都铺满工作区），切换靠**左栏「连接」区**（会话列表下方，每个窗口一行）
  与状态栈「连接」chip 的列表；两处读同一份名单，见 §13。
- 连接身份 `key` **恒等于 window label**。这是整套协调面的地基。
- 白名单与 Bearer 都按 window label 分槽。

## 2. 为什么不做一个窗口里的连接列表

TUI 做得到：它的状态本来就按 lane 分区（`LaneApp` 各自持有 `TuiApp`）。

Desktop 做不到同样的形状，因为业务 UI 的状态是 **12 个 `thread_local` 单例**
（`sse_dispatch` 的 pending id、tool-call 映射、overlay 光标、上传源……）。单窗口多连接
意味着按连接给每一个分区 —— 那是把该 crate 里 `thread_local` 的语义改掉，并且要把
`crabmate-connect` 改成多会话客户端。

多窗口让 `thread_local` 天然就是「本窗口的」，两处都不用改。

这不是新方向：`tui_multi_server.md` §9.4.1 在本功能存在之前就推荐了多 WebView 窗口，
能力矩阵当时也是这么写的。这次是把既有结论落成实现。

## 3. 协调面

| 层 | 内容 |
|---|---|
| 契约（纯逻辑） | `crabmate_client_api::connections`：`ConnectionKey` / `ConnectionSnapshot` / `ConnectionStatus` / `ConnectionTally` |
| 壳侧注册表 | `desktop-tauri/src-tauri/src/desktop_connections.rs`：`ConnectionRegistry`（`label → snapshot`）+ `connections_changed` 事件 |
| 前端读取 | `frontend/src/app/status_connection_chip.rs`：一份应用级 `ArcRwSignal`，先 `register` 再 `list` 再 `listen` |

三条命令构成对外面：

| 命令 | 谁调 | 作用 |
|---|---|---|
| `open_connection(api_base, display_name?, bearer?)` | 前端 | 分配 label、建窗、登记、广播 |
| `register_connection(api_base, display_name?)` | 每个窗口自报 | 把自己放进名单（否则 main 永远缺席） |
| `focus_connection(key)` | 前端列表点击 | 切到某窗口 |

**`register_connection` 必须有调用者。** 只有 `open_connection` 会登记时，main 窗口自己
不在名单里 —— `len` 永远凑不到 2，chip 就不会切成列表，功能在用户眼里等于没生效。
这是实现期真实踩到的坑，见 §7。

## 4. `key` 恒等于 window label

```rust
snapshot.key = ConnectionKey::from_raw(window.label());
```

`from_raw` 只做 slug 化，label 本身已经合规，所以这个等式成立。

**为什么是等式而不是「用 key 找 label」**：`focus_connection`、`report_connection_status`、
`rename_connection`、窗口 `Destroyed` 清理**全部按 label 查**。若 `key` 是另一个值，
`focus_connection(key)` 就会找一个不存在的 label。

只有 `open_connection` 需要**分配**新 label（窗口还不存在），那一步用
`unique_key(可选名字, 基址)`，撞名时依次尝试 `-2`、`-3`。

## 5. 几何、遮挡与审批通知

窗口都铺满工作区（复用 `apply_main_ui_geometry`：maximize，失败则填满工作区）。
后开的自然盖住先开的 —— 视觉上像切标签，实现上各自独立。

**遮挡带来了一个必须补的能力**：被盖住的窗口看不见自己的审批框。所以
`report_connection_status` 在窗口进入 `WaitingApproval` 且**它不在前台**时发一条系统通知
（`tauri-plugin-notification`）。没有这条，后台窗口的审批会一直静默，直到用户碰巧切回来 ——
比单窗口时更糟，因为单窗口至少能看见。

通知**不撤不并**：桌面侧插件不暴露 notification id，做不了「同一连接升级同一条」。这一点
与 ADR-0002 的 Android 侧不同，那边有 `notificationId`。

几何必须在 `show()` **之后**应用：X11 / Wayland 上未映射时 `maximize` 与
`current_monitor` 都不可靠。

## 6. 白名单与 Bearer：都按 label 分槽

两者是**同一个问题**，第一个早就修了，第二个当时漏了。

| 项 | 形状 |
|---|---|
| `AllowedServeOrigin` | `label → origin`；导航判定用**发起导航的那个窗口** |
| Bearer（钥匙串） | account = `<原账户名>:<label>` |

`AllowedServeOrigin` 的注释写着「进程级单值会让先连的那个窗口丢掉放行依据」—— Bearer 是
当时没跟上的另一半。

**Bearer 串号的真实后果不是「重启后串」**：窗口 A 点断开就清空了进程级钥匙串，窗口 B
**下一次刷新**就认证失败。不需要重启。

**迁移**：旧版只有一个不带 label 的槽。新槽为空时回退读旧槽，命中就搬进新槽并**删掉旧槽**。
删是必要的 —— 留着会把同一份凭据分发给每一个窗口。迁移只在真实首次启动时执行，
CI 与单测都碰不到。

## 7. 实现期踩到的三个坑（都是读出来的，不是跑出来的）

| 坑 | 症状 | 教训 |
|---|---|---|
| `register_connection` 用 `unique_key` 派生 key、却拿 `window.label()` 当注册表键 | 窗口都正常、测试全绿、功能**静默无效** | 单元测试证明不了「意图接通了」 |
| 没人调用 `register_connection` | main 不在名单里，列表永远只有 1 条 | 接口存在 ≠ 接口被用 |
| 前端 `INSTALLED` 是进程级标志、`connections` 是每实例 signal | 状态栈重挂载后列表冻结在空 | 生命周期不一致会静默 |

第三个的修法是**换所有权模型**而不是补 `on_cleanup`：名单描述的是应用级状态，就该是应用级
signal（`thread_local OnceCell` 懒建一份）。订阅与 signal 同寿，于是「回调 `set` 已 drop 的
signal」这个问题**不存在**，而不是被兜住。

中途试过 `on_cleanup + Rc<RefCell>`，被 `Send + Sync` 挡回两次 —— 那是方向错误的信号：
把应用级状态塞进组件生命周期，就注定要处理「async 先完成 vs cleanup 先到」的竞态。

## 8. capabilities 的 `windows: ["*"]`

连接窗口的 label 在运行期分配，无法编译期枚举，所以不能像以前那样列 `["main", "default"]`。

- **放宽的是**「哪些本地窗口可以用这些插件」，不是「能连哪些远端」—— `remote.urls` 仍只
  允许回环。
- 不放宽，第二个窗口连 `invoke` 都用不了。

## 9. 生命周期

- 开窗在**注册之前**：建窗失败就不该在列表里留一条打不开的连接（占位也一并回滚）。
- `open_connection` 先**占位**再建窗：`unique_key` 返回时锁已释放，两个并发开窗会拿到同一个
  key，而 `WebviewWindowBuilder` 遇重名 label 会失败 —— 表现为「偶发开不出来」。
- **关窗 = 注销**（`Destroyed`），不做 keepalive：常驻通道（ADR-0002）与「多余的窗口」是相反
  诉求，同时启用会让后台工作加倍。

## 10. 已知限制

- 通知不撤不并（桌面插件无 id / 撤销 API）。
- `snapshots()` 广播时强制 `active = false`，而 `open_connection` / `register_connection`
  的返回值里是 `true` —— 前者给「别人」看（列表里谁是焦点不归注册表判），后者给调用方看
  「你自己」。两套口径是有意的，但容易误读。
- **本功能尚未经过真实多窗口运行**（实现环境无 `serve`、无图形会话）。

## 11. 冒烟步骤

需要可用的 `crabmate serve` 与图形环境（步骤汇总见 [`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）。

1. **设置 → 连接 → 新建连接窗口** → 应新建一个停在连接页的窗口（标题栏 `CrabMate · 名称`）。
   在那里填地址连上后，两个窗口的左栏「连接」区都应出现两行，当前窗口那行高亮。
2. 开两条连接 → 第二个窗口应盖在 main 上，标题栏带连接名。
3. 状态栈「连接」chip 应变成 2 条；点条目应切到对应窗口；**左栏那两行点也应切窗口**。
4. **窗口 A 断开 → 窗口 B 刷新后仍应能认证**（这条专验 Bearer 分槽）。
5. 窗口 B 触发审批 → 切到 A → 应收到系统通知，且 B 那行状态字形应变 `▲`。

第 4 步最值得跑：它验的是迁移与分槽，而这两块 CI 完全覆盖不到。

## 12. 不做

- **单窗口内并排多 lane**（同屏同时看两份 transcript，理由见 §2；§9.4.2 明确禁止为「对齐」而加）。
  注意与 §13 的区别：左栏连接区是**导航器**（同一时刻只显示一个窗口的会话），不改变
  「一个窗口一份 JS 堆」这条前提。
- 跨窗口共享会话状态（各窗口的 `thread_local` 就该各管各的）。
- Android 多连接：没有窗口管理器，不适用。

## 13. 左栏「连接」区（导航器）与「新建连接窗口」入口

TUI 在左栏下半画 lane 区（`tui_multi_server.md` §4.6），Desktop 现在在同一位置画
**连接导航**：会话列表下方常驻，每个窗口一行（实现：`frontend/src/app/nav_rail_connections.rs`）。

| 项 | 设计 |
|---|---|
| 数据面 | 复用 `status_connection_chip::connection_feed` —— 应用级 signal，订阅只装一次，与右列 chip 读同一份 |
| 何时显示 | 桌面壳且名单非空。**单连接也显示**（对齐 TUI：不显示就看不出连的是谁，也看不出支持多连接） |
| 点击 | `focus_connection(key)` → 切到那个窗口（换整个 JS 堆，不是切 lane） |
| 单连接 | 行内按钮 `disabled`：没有别的窗口可切，而「改基址」归右列 chip，两个入口不重复 |
| 高亮 | 按 `authority` 与自己的 `api_base` 比对 —— 广播的 `snapshots()` **强制** `active = false`（同一份 payload 不可能对每个窗口都成立）。唯一会让两行同时亮的场景（两个窗口连同一台 serve）已由连接时去重挡住，见下节 |
| 状态字形 | `·` 空闲 / `●` 运行中 / `▲` 等审批 / `×` 已请求退出（与 TUI 同口径，呼吸动画只在终端） |
| 窄屏 | 不需要额外媒体查询：DOM 在 `.nav-rail` 内，抽屉打开时自动跟着进去 |

**这不是 §12 说的「单窗口内并排多 lane」。** 同一时刻只显示一个窗口的会话；行是导航，
不是分区视图。

### 新建连接的入口

「设置 → 连接」新增 **新建连接窗口**（命令 `open_connect_window`）：

- 只建一个载 `connect.html` 的窗口，**连哪台 serve 由用户在新窗口里选**（`connect_remote`
  早已按 `window.label()` 写白名单与 Bearer 槽，所以新窗口点「连接」天然可用）。
- 因此按钮**不读**旁边「保存基址」的 draft —— 那个 block 的语义是「改**当前窗口**连谁」，
  吃同一个 draft 会让两个动作看起来是一回事。
- 建窗前先 `unique_key` **占位**：`unique_key` 只看注册表，不占位时连点两次会拿到同一个
  label，`WebviewWindowBuilder` 遇重名直接失败（与 `open_connection` 同一理由）。
- 占位那条在新窗口连上前是**空基址**（chip 的单条分支对空基址整段不渲染；多连接列表里
  显示为无名行）。连上后由业务 UI 的 `StatusConnectionChip` 调 `register_connection` 覆盖。

### 同一台 `serve` 只连一个窗口（连接时去重）

一个 serve 不需要两个窗口 —— 两个窗口连同一台时，注册表里会有两条相同 `authority`
的记录，「切到哪一条」无从判断（左栏两行都会高亮，见上文局限）。口径与 TUI 的
`/lane attach` 一致：**同一个 `base_url` 上换一个名字只切过去、不新增 lane**。

拦截点在 `connect_remote` 的 `probe_server` **之后**：

| 项 | 设计 |
|---|---|
| 时机 | 探测成功之后。地址都不可达时不该先谈「谁占着」——那会把「连不上」误报成「已占用」 |
| 不在开窗时拦 | 「新建连接窗口」**不带**地址（B 案），开窗那一刻没有基址可比 |
| 命中后 | `focus_connection` 的同类动作（`unminimize` + `set_focus`）切到已有窗口；本窗口**留在连接页**并返回错误，所以「为什么没进去」写在用户正在看的那一屏 |
| 判据 | `authority`（`host:port`）相等且不是自己。尾斜杠、scheme 大小写因此不算不同 |
| 空基址 | 一律不算命中：占位条目（建窗后、连上之前）的 `authority` 就是空的 |
| 依赖方向 | 注册表在 `desktop-tauri`、命令在 `crabmate-connect`，方向是 desktop → connect，所以**不能反向引用**。`ExistingConnectionProbe` 是一个闭包壳（`crates/crabmate-connect/src/existing_connection.rs`），壳在 `setup` 里 `manage` 一份；命令取不到该 state（浏览器 / Android / 老壳）就当作无冲突——那些壳本来只有一条连接 |
