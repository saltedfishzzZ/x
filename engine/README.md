# 类地行星地形核心

当前目录保留 **E0、E1、E2-v1 与 E2.1-v2** 的冻结资产。阶段审计确认 E2-v1 可以稳定重现既有原型，但其身份隔离、M1 语义验证和证据门槛不足以直接进入 E3，因此标记为 `FROZEN_LEGACY_PROTOTYPE`。E2.1-v2 已经完成双平台验证并正式冻结；E3 按当前决定暂停。

E2-v1 构造／M1 最小垂直切片已经通过 macOS Apple Silicon 与 Windows AMD64 原生验收及跨平台比较，包含：

- 共享身份、状态和资产契约；
- 等角立方球面及四叉身份实验；
- 命名随机、固定归约和最小球谐实验；
- 平台感知拓扑实验；
- 分片线性容器积分实验；
- 双摘要、字段块和原子发布实验；
- E1 等角立方球面空间发布；
- E2 世界身份、命名随机和主板块球面分区；
- E2 共享边界、欧拉运动和四类边界资格；
- M1 输入束、球面垂直基准和受限准入；
- 无界面诊断 CLI 与独立 Python Oracle。

E2 普通世界只允许诊断，PV01 固定世界只允许测试命名空间。材料、正式高程、海陆、气候、水文和桌面应用尚未实现；`terrain_generation` 继续为 `NOT_AVAILABLE`。

E2.1 使用独立的 `fixtures/e2.1` 与 v2 模式文件。旧 `fixtures/manifest.json`、E2-v1平台报告和冻结报告属于受保护的历史基线，不在E2.1中原地修改。

本机检查：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p terrain-cli -- inspect
cargo run -q -p terrain-cli -- diagnostics
cargo run --release -p terrain-cli -- tectonics conformance --seed PV01-E2-v1 --provenance fixed-test
cd reference/python && python3 -m unittest discover -s tests -v
```

统一平台验证及报告生成：

```bash
python tools/verify_platform.py
python tools/verify_e1_platform.py
python tools/verify_e2_platform.py
python tools/verify_e2_1_platform.py
```

Windows 可使用 `python` 执行同一脚本。E0／E1 使用
`tools/compare_platform_reports.py` 比较离散证据、规范摘要和连续误差。

E2 的 Windows 原生报告生成命令为：

```powershell
python tools\verify_e2_platform.py --output reports\e2\platform-windows-amd64.json
```

Python 最低支持版本为 3.11。E2 两端报告与冻结比较结果已归档在 `reports/e2`；权威证据摘要为 `063e75b247b3ecb3831af09a5920522361bd804dba71568ce51e0a151ce19847`。

E2.1两端报告、跨平台比较、证据包和冻结报告归档在`reports/e2.1`。E2.1冻结源码树摘要为`41361af3ed1805e1a52754539cbf28be1a0e7b9740d8d4c05ded121a71fdf4ea`。

权威设计见工作区根目录的中文文档导航。
