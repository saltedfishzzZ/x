# 类地行星地形引擎

这是一个以种子驱动、可重现和成因可解释为目标的类地行星自然状态引擎项目。项目采用独立引擎核心与桌面应用分离的结构，计划支持 Windows 和 macOS，并从全球球体连续缩放到区域与地貌尺度。

## 当前状态

- E0 工程底座、E1 球面空间层、E2-v1 构造原型和 E2.1-v2 修正版均已冻结。
- E2.1-v2 是当前权威构造与 M1 输入基线，macOS 与 Windows 原生验证一致，E21-G01 至 E21-G20 全部通过。
- 冻结源码树摘要：`41361af3ed1805e1a52754539cbf28be1a0e7b9740d8d4c05ded121a71fdf4ea`。
- E2.1 证据包规范摘要：`fefc7e7c2192c4d80a26984b9136ad6efa6c9553e18c7d9b2ebd2fce438d8f83`。
- E3 当前暂停。正式材料、高程、海陆、气候、水文、自然地貌和桌面应用尚未实现。

## 阅读入口

- [文档导航](文档导航.md)：全部设计、项目管理、验收与历史文档的索引。
- [当前共识与待定事项](文档/项目管理/当前共识与待定事项.md)：当前权威范围、边界和后续顺序。
- [引擎目录说明](engine/README.md)：工程结构、冻结范围和本地检查命令。
- [E2.1 冻结验收报告](engine/reports/e2.1/E2.1冻结验收报告.md)：最终双平台证据与冻结结论。

## 目录结构

```text
.
├── engine/       Rust 引擎、固定件、规范、验证工具与冻结报告
├── 文档/          中文设计、项目管理、验收、专题与历史记录
├── README.md     仓库入口
└── 文档导航.md    文档索引
```

## 本地只读检查

以下命令不会覆盖仓库中的冻结平台报告：

```bash
cd engine
cargo fmt --all -- --check
python3 tools/verify_e2_1_platform.py --output /tmp/e2.1-local-report.json
```

Windows PowerShell 可将报告写入临时目录：

```powershell
cd engine
python tools\verify_e2_1_platform.py --output "$env:TEMP\e2.1-local-report.json"
```

冻结发布语义为 `ATOMICALLY_VISIBLE + DURABILITY_NOT_PROVEN`。任何改变冻结契约或权威结果的工作都应建立新兼容版本，并重新生成双平台证据，不应原地改写冻结资产。
