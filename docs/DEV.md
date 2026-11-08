# 帧析助手 · 主版本（Win10 / Win11）—— 零基础说明

这个目录是 **Rust 版的主开发目录**（在 `serial_udp\app_rust\frameanalyzer\`）。你不需要懂 Rust，只要记住三件事：

> **改代码在 `src\` → 出包双击 `build_main.bat` → 运行双击本目录的 `frameanalyzer.exe`**

> 📌 **路径怎么看**：带 `app_rust\...` 前缀的路径都从项目根 `serial_udp\` 算起；不带前缀的（`src\`、`target\`、`.config.json`）都从本文档所在目录算起。
> 项目根 `serial_udp\` 放的是**两版共用的东西**：`app.ico`、`测试数据.txt`、`README.md`。Python 版归在 `app_python\`，Rust 版归在 `app_rust\`。

Win7 专用版在**隔壁** `app_rust\frameanalyzer-win7\`（相对本目录就是 `..\frameanalyzer-win7\`），本目录保持干净、不受它影响。

---

## 1. 源码是哪些文件

### 真正要改的：只有 `src\` 这一个文件夹（约 194 KB）

| 文件 | 干什么 |
|---|---|
| `src\main.rs` | 程序入口：建窗口、设标题、加载图标 |
| `src\frame_sync.rs` | **最核心**：分帧、帧头对齐、SUM 校验、无帧头时连续两帧锁定、CSV 保存 |
| `src\extract.rs` | 从一帧里按字节序/数据类型提取各通道数值 |
| `src\link.rs` | 串口 / UDP 收发 |
| `src\config.rs` | 配置的数据结构 |
| `src\ui\app.rs` | 主界面：左栏控件、状态行、帮助窗口、配置读写 |
| `src\ui\i18n.rs` | 中英文词条 |
| `src\ui\plot.rs` | 波形绘制 |
| `src\ui\style.rs` | 配色、间距、字体 |
| `src\lib.rs` / `src\ui\mod.rs` | 模块登记（相当于目录索引，一般不改） |
| `src\bin\parity.rs` | 对拍小工具，验证 Rust 与 Python 版算法一致 |
| `tests\` | 自动测试（目前 53 项） |

### 不是源码，但**必须留着**

| 文件 | 相当于什么 | 能不能删 |
|---|---|---|
| `Cargo.toml` | 依赖清单（"要用到哪些现成库"） | ❌ 别删 |
| `Cargo.lock` | 每个库的**精确版本号**（锁定现场） | ❌ **千万别删、别手动改**，Win7 版能不能编出来全靠它 |
| `build.rs` | 编译前的小脚本：把图标嵌进 exe | ❌ 别删 |
| `app.ico` | 程序图标，**在项目根 `serial_udp\`（本目录上两级）**，Python 版打包也用这一份 | ❌ 别删，删了两版都编/打不过 |
| `build_main.bat` | 一键出包 | ❌ 别删 |
| `verify_release.py` | 出包后自检"这是哪套包、SHA256 多少" | ❌ 别删 |
| `target\` | **编译产物 / 缓存**，几 GB | ✅ 可随时删，下次编译自动生成 |

> 一句话：`target\` 删了什么都不影响，`src\` 删一行程序就变了。

---

## 2. 构建命令是什么

### 最省事：双击 `build_main.bat`

它会自动做五件事，并在窗口里打印结果（双击时结尾会停住等你按键）：

1. 显示工具链版本 → `cargo 1.98.x`
2. 编译 → `cargo +stable build --release`
3. 自检产物 → 读 PE 导入表，打印大小、SHA256、判定"这是 Win10/11 版"
4. **把 exe 复制到本目录** → 逐字节比对确认复制无误，再对副本自检一次
5. **删掉整个 `target\`** → 只留 6 MB 的成品，不留下几 GB 缓存

> ⚠️ 任何一步失败（编译失败 / 自检不通过 / 复制失败）都会**保留 `target\` 不删**，避免连旧 exe 一起丢掉。
> 也因为每次都会删 `target\`，**每次打包都是全量编译，约 2 分钟**，这是用磁盘空间换来的，属预期行为。
> 编译若遇到 `拒绝访问 (os error 5)`（全新 `target\` 首次创建时安全软件偶发锁文件），脚本会**自动重试一次**，通常几秒就好。
>
> ⚠️ **不要同时双击两个 bat**：主目录和 Win7 目录都是全量编译，并行会互相抢资源导致长时间卡住（实测 17 分钟无进展）。**一个跑完再跑另一个**，各自约 2 分钟。

### 想手敲命令（在 `app_rust\frameanalyzer\` 目录里开终端）

```bat
cargo +stable build --release
```

编译产物先在 `target\release\`，但**打包脚本会自动把 exe 复制到本目录、然后删掉整个 `target\`**。
所以你要找的成品是 **本目录下的 `frameanalyzer.exe`**（不是 target 里面那个）。

几个词的意思（不用背）：
- `cargo` = Rust 的"构建管家"，负责下载依赖、编译、跑测试
- `+stable` = 明确用最新版编译器（本目录没有强制工具链文件）
- `--release` = 出正式版（体积小、跑得快）；不加就是调试版，会写进 `target\debug\`
- 第一次编译要 2~3 分钟，之后只重编改动的部分，几秒到十几秒

### 其它常用命令

| 想做什么 | 命令 |
|---|---|
| 编译并**直接启动** | `cargo +stable run --release` |
| 跑自动测试 | `cargo +stable test --lib` |
| 跑测试并看打印内容 | `cargo +stable test --lib -- --nocapture` |

---

## 3. 怎么启动

**双击** `app_rust\frameanalyzer\frameanalyzer.exe`。

启动后程序会在 **exe 所在目录**自动生成/读取这些文件，它们都不是源码：

| 文件 | 内容 | 删了会怎样 |
|---|---|---|
| `.config.json` | 语言、主题、帧配置、通道配置、连接参数、命令列表 | 自动重建，但**你的设置回默认值** |
| `ui_style.json` | 界面微调（左栏宽度） | 同上 |
| `extracted.csv` / `raw.txt` | 你手动导出的数据 | 就是数据文件，删了没了 |

---

## 4. 打包 Win7 版，要自己手动复制源码吗？

**不需要，一个文件都不用手动复制。**

双击 `app_rust\frameanalyzer-win7\build_win7.bat`，它的第 1 步就会自动把主目录的 `src\`、`tests\`、`build.rs` 整个镜像过去，并自动生成 Win7 版专用的 `Cargo.toml`，然后才构建。

实测验证过：在主目录 `src\` 里放一个临时标记文件 → 跑同步 → 它自动出现在 Win7 目录；把主目录那个删掉 → 再跑同步 → Win7 目录里也自动消失。是**全量镜像**，不是只增不删。

> ⚠️ **反过来要记住**：`app_rust\frameanalyzer-win7\src\` 里永远别手改代码，改了下次同步就被覆盖。所有改动只在主目录做。

Win7 版为什么要用老编译器、为什么钉了几个依赖版本，详见 `app_rust\frameanalyzer-win7\README.md`。

---

## 5. 想放 GitHub 共享，推哪些文件？

### 先说一个容易踩的坑

Git 推的**不是"一个文件"，而是一整个目录树**。而且要在 **`serial_udp\` 这一层**（也就是 `app_rust\frameanalyzer\` 的上两级）执行 `git init`，**不要**在 `app_rust\frameanalyzer\` 里面 init —— 因为共用的 `app.ico` 在最外层项目根，在里面开仓库会把它漏掉，别人 clone 下来直接编译失败。

项目根已经放了一份 `.gitignore` 帮你挡掉垃圾，先确认它在，再 `git add .`。

### ✅ 应该推

```
.gitignore                              ← 排除规则本身
README.md                               ← 项目使用说明（可选）
app.ico                                 ← ★ 共用图标，千万别忘了
测试数据.txt / 测试数据1.csv             ← 共用测试数据

app_python/                             ← Python 版（源码 + build.bat + 打包命令.txt）
                                           帧析助手back\ 里是 Python 历代版本备份

app_rust/源码v1_校验功能前/              ← Rust 历史源码快照（冻结存档，不参与构建）
app_rust/源码v2_校验功能/

app_rust/frameanalyzer/
    Cargo.toml  Cargo.lock  build.rs    ← ★ Cargo.lock 一定要推
    src\         tests\
    build_main.bat  verify_release.py
    README.md                           ← 本文档

app_rust/frameanalyzer-win7/
    Cargo.toml  Cargo.lock  build.rs
    rust-toolchain.toml                 ← ★ 靠它固定 1.77.2
    .cargo\config.toml                  ← ★ 靠它静态链接运行库
    src\         tests\                 ← 可以由同步脚本重新生成，但推上去别人开箱即用
    sync_from_main.py  check_win7_imports.py  build_win7.bat  README.md
```

**为什么 `Cargo.lock` 必须推**：它记着每个依赖的精确版本。Win7 版被钉在 `winresource 0.1.22`、`immutable-chunkmap 2.0.6` 等一批老版本上，没有 lock 别人一解析就会拉到新版，`1.77.2` 直接编不过。

### ❌ 绝对不要推（`.gitignore` 已自动挡掉）

| 内容 | 体积（本机实测） |
|---|---|
| 两个 `target\` | **约 5.2 GB**（debug 3.3 G + release 1.9 G） |
| `_nuitka_temp.build\` / `.onefile-build\` | 875 MB（老 Python 版打包残留） |
| 各种 `.exe` / `.zip` / `.rar` | 根目录约 100 MB 历史版本 + 两个项目目录里刚打出来的成品 exe |
| `_parity_*.json` | 3.7 MB 诊断数据 |
| `.config.json` / `ui_style.json` / `extracted.csv` / `raw.txt` | 个人设置和导出数据 |
| `__pycache__\` / `.vscode\` / `Thumbs.db` | 缓存和编辑器文件 |
| `serial_and_udp_tool\` | 内嵌的独立 git 仓库 |

### 想让别人直接下载 exe

**不要把 exe commit 进仓库**（二进制文件进 Git 会让仓库越滚越大、且没法 diff）。用 GitHub 仓库页面的 **Releases → Create a new release**，把 exe 作为附件上传，写清楚哪个是 Win7 版、哪个是 Win10/11 版。两个 exe 名字一样，附件名建议改成：

- `frameanalyzer-win7.exe`
- `frameanalyzer-win10-11.exe`

（本机上仍然保持原名，只在上传附件时改名，避免搞混。）

---

## 6. 磁盘占用：`target\` 现在由脚本自动清理

**你基本不用手动删了。** 两个打包脚本在成功出包后会自动删掉整个 `target\`，只把 6 MB 的 `frameanalyzer.exe` 留在各自项目目录里。打包过程中 `target\` 会临时涨到几 GB，结束后消失。

| 情况 | 怎么办 |
|---|---|
| 打包正常结束 | 不用管，`target\` 已被删 |
| 打包失败 | 脚本**故意保留** `target\` 方便排查，修好后重跑即可 |
| 想手动清 | 直接删整个 `target\`，源码一行不丢；成品 `frameanalyzer.exe` 在**上一层目录**，不会被误删 |
| 以前残留的 `target\` | 已由脚本清掉；若还有，手动删即可 |

**永远别删**：`src\` `tests\` `Cargo.toml` `Cargo.lock` `build.rs`、`app.ico`、Win7 目录的 `rust-toolchain.toml` / `.cargo\config.toml` / `Cargo.lock`、以及根目录的 `.config.json` / `ui_style.json`（你的界面设置，删了回默认值）。

> 代价：因为每次都从零编译，双击 bat 到出包约 **2~3 分钟**。依赖包缓存在本机 `~/.cargo`，不需要重新下载。

---

## 常用动作速查

| 我想… | 做这个 |
|---|---|
| 改功能 | 只改 `app_rust\frameanalyzer\src\` 里的文件 |
| 出 Win10/11 包 | 双击 `app_rust\frameanalyzer\build_main.bat` |
| 出 Win7 包 | 双击 `app_rust\frameanalyzer-win7\build_win7.bat`（会自己同步源码） |
| 直接运行 | 双击对应项目目录下的 `frameanalyzer.exe` |
| 验证改坏了没 | 双击 `app_rust\frameanalyzer\test_baseline.bat`（等价于 `cargo +stable test --lib`，额外打印源码指纹、通过/失败数与耗时，跑完自动删 `target\`；`--all` 可连 Win7 版一起串行跑） |
| 清理磁盘 | 不用手动做：打包成功后脚本自动删 `target\`，只留本目录的 `frameanalyzer.exe` |
| 传 GitHub | 在 **`serial_udp\`** 层 `git init`，确认 `.gitignore` 在，再 `git add .` |
