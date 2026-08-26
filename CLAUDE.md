# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概览

Lance Graph 用 Rust 实现了一个支持 Cypher 的图查询引擎:把 Lance/Arrow 数据集解释为属性图,将 Cypher 查询翻译成 DataFusion SQL 执行;同时提供直接 SQL(`SqlQuery`/`SqlEngine`,无需 `GraphConfig`)。Python 侧由同一引擎提供 `lance_graph` 包,以及构建在它之上的 `knowledge_graph` 包(CLI + FastAPI + Lance 存储)。

结构为 Cargo workspace(root `Cargo.toml`),共 4 个 crate:

- `crates/lance-graph` — 引擎核心(parser/semantic/logical plan/physical planner/执行)
- `crates/lance-graph-catalog` — 目录抽象:InMemory、目录命名空间(DirNamespace)、Unity Catalog、Delta/Parquet 表读取,供 planner 和 catalog 扩展复用
- `crates/lance-graph-benches` — 基准 crate(不发布,bench 均放这里,不要加进主 crate)
- `crates/lance-graph-python` — PyO3 绑定(cdylib,模块名 `_internal`),**被 workspace exclude**,由 maturin 单独构建

文档:详细设计见 `docs/project_structure.md`(注意:这是 issue #92 的**提案**,其中拆分 `lance-graph-core`/`lance-graph-planner` 尚未实现,勿按它断言现状)、`python/CLAUDE.md`(Python 侧速查)、`python/DEVELOPMENT.md`(部分内容继承自 Lance 项目,S3/dynamodb 集成、tracing shell 等章节与本项目无关)。

## 常用命令

### Rust(crate 位于 `crates/lance-graph`,加 `--manifest-path` 或在目录内直接跑)

```bash
cargo check --manifest-path crates/lance-graph/Cargo.toml
cargo test --manifest-path crates/lance-graph/Cargo.toml --lib      # 单元测试
cargo test --manifest-path crates/lance-graph/Cargo.toml --tests    # 集成测试(tests/*.rs)
cargo test --manifest-path crates/lance-graph/Cargo.toml --doc      # 文档测试
# 单个测试(在 crates/lance-graph 下):
cd crates/lance-graph && cargo test datafusion_scenarios
# 代码检查(CI 要求 warnings 为 0):
cargo clippy --manifest-path crates/lance-graph/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path crates/lance-graph/Cargo.toml -- --check
# 基准(bench 在 lance-graph-benches crate,从 workspace 根跑):
cargo bench --bench graph_execution -p lance-graph-benches
cargo bench --bench graph_execution -p lance-graph-benches -- --warm-up-time 1 --measurement-time 2 --sample-size 10
```

Linux 构建/测试需 `sudo apt-get install -y protobuf-compiler`(CI 如此配置)。

### Python(在 `python/` 目录内)

```bash
# 首次初始化
uv venv --python 3.11 .venv && source .venv/bin/activate
uv pip install 'maturin[patchelf]'
uv pip install -e '.[tests]'

maturin develop    # 编译 Rust 扩展;改 Rust 后必须重跑,纯 Python 改动不需要

make test          # 等价于 uv run pytest python/python/tests/test_graph.py(只有这一个文件!)
# 跑全量测试:
uv run pytest -v python/python/tests
# 单个测试:
uv run pytest python/python/tests/test_graph.py::test_basic_node_selection -v

make lint / make format       # ruff + pyright / ruff format
make doctest / make clean
```

注意测试路径是双层 `python/python/tests`(从 `python/` 目录看)。`make test` 只跑 `test_graph.py`,全量请直接用 pytest。

## 架构:查询管线

核心流程(见各模块开头的注释):

```
query.rs:CypherQuery::new(query)
  → parser.rs:parse_cypher_query        (nom 手写解析器,产出 ast.rs 的 AST)
  → semantic.rs:SemanticAnalyzer        (校验 + 变量作用域/类型追踪 + 参数替换,大小写不敏感)
  → logical_plan.rs:LogicalPlanner      (LogicalOperator 枚举:ScanByLabel/Filter/Expand/VariableLengthExpand/Unwind…)
  → datafusion_planner/:DataFusionPlanner(转成 DataFusion LogicalPlan)
  → DataFusion SessionContext 执行       (表数据经 table_readers.rs:Parquet/Delta/Lance)
```

`datafusion_planner` 是两阶段(见 `mod.rs` 注释):

1. **analysis.rs** — 为关系实例分配唯一 ID、收集变量 → 标签映射、所需数据集;
2. **builder/** — 逐 operator 建 DataFusion 计划(scan/join/aggregate/expand/vector 等 ops)。

关键约定:
- 计划中所有列名整理为 `{variable}__{column}`(如 `p__name`、`r__weight`)避免歧义;schema/RecordBatch 统一小写化(`query.rs::normalize_schema`)。
- 变长路径 `*1..3` 通过展开成多个固定长度计划 + UNION 实现,上限 `MAX_VARIABLE_LENGTH_HOPS = 20`(lib.rs)。
- `to_sql`/`to_spark_sql`(`query.rs` + `spark_dialect.rs`)把 Cypher 转成 SQL 字符串,支持 5 种方言(`SqlDialect`);测试在 `tests/test_to_sql.rs`。
- 错误用 snafu:统一 `GraphError { message, location }` / `Result`(error.rs),上下文经 `location: snafu::Location` 追溯。
- 语义统一不区分大小写(`case_insensitive.rs::CaseInsensitiveLookup`)。

## 特性开关与配置

- `crates/lance-graph` 默认特性:`unity-catalog`、`delta`(deltalake 依赖较重);lib.rs 对它们有 gated re-export,但内部可能无 feature gate 的路径,加 feature 条件时留意。
- `GraphConfig`(config.rs)是图语义的入口:`.with_node_label()`/`.with_relationship()` 声明节点/关系表的列映射,再经 `CypherQuery::with_config` 传入;缺 config 时仅能跑 SQL 类查询。
- planner 依赖 `GraphSourceCatalog`(crates/lance-graph-catalog,发布的 crate 名为 `tf-lance-graph-catalog`)加载表:`InMemoryCatalog`、`DirNamespace`(目录/本地文件)、Unity Catalog(可注册 Delta/Parquet,见 README 示例)。

## Python 绑定

- `crates/lance-graph-python` 用 PyO3 暴露 `_internal` 模块(`graph` 子模块);`python/python/lance_graph/__init__.py` 是门面,从 `_internal` re-export 公共类型。
- 门面自带 **dev fallback**:找不到已安装扩展时,会扫描 `python/target/` 与 repo 根 `target/` 下的 `_internal*.so` 并加载——所以 `maturin develop` 后即使不 `pip install` 也能 `import lance_graph`(加载的是 target 里最新产物;改 Rust 后忘了重跑会导致用了旧扩展)。
- `knowledge_graph` 包:`uv run knowledge_graph`(CLI,默认用 OpenAI LLM 抽取,`--extractor heuristic` 回退到启发式)、`uv run --package knowledge_graph knowledge-graph-webservice`(FastAPI,/graph/* 端点);数据默认落在 `./knowledge_graph_data`。存储/localds 逻辑在 `store.py`,测试见 `python/python/tests/test_store.py`。

## 测试布局与 CI

- Rust:单元测试写在模块内 `#[cfg(test)]`;集成测试在 `crates/lance-graph/tests/*.rs`(覆盖 DataFusion 场景、向量检索、to_sql、大小写、EXPLAIN 等);catalog crate 有 `tests/unity_catalog_integration.rs`。
- Python:`python/python/tests/test_*.py`;pytest markers 定义于 pyproject(`integration`/`slow`/`gpu` 等)。
- CI(.github/workflows):`style.yml` = fmt(`cargo fmt --all -- --check`)+ clippy(`cargo clippy --workspace --all-targets -- -D warnings`)+ typos;`rust-test.yml` = workspace 级 `--lib`/`--tests`/`--doc` 三挡;`python-test.yml` = Python 3.11 + maturin + pytest。改 `crates/` 下 Rust 代码时,需保证本地这些命令全绿,否则 CI 必挂。

## 版本与发布

- 版本号由 `ci/bump_version.py` + `.bumpversion.toml` 管理,一次同步 4 处:`crates/*/Cargo.toml` ×3 和 `python/pyproject.toml`(bump 后脚本自动 `cargo check` 刷新两个 Cargo.lock)。当前 0.5.5(以 `.bumpversion.toml` 为准)。
- 发布流程在 `release.yml` / `rust-publish.yml` / `python-publish.yml`(maturin build + PyPI/crates.io)。
- Commit message 跟随历史风格(`feat(graph):`、`docs:`、`refactor(query):` 等,详见 AGENTS.md)。
