# Codex CLI: внутренние механизмы и проверка жизненного цикла

Проверено 1 ноября 2026 года на официальном Linux x86_64 musl CLI **0.166.0**.
Это исследование открытого CLI/app-server; закрытый Codex Desktop не запускался.

Исходники: тег [`rust-v0.166.0`](https://github.com/openai/codex/releases/tag/rust-v0.166.0),
commit `6ff1e1ca3b30fc8b914720681fcf5fe62e95f856`.
Архив `codex-x86_64-unknown-linux-musl.tar.gz` проверен по SHA256
`a0bf572d2f93f056c6712a6da348e96078dc84f3885fd7be67807ef389cb3978`;
SHA256 извлечённого бинарника — `b8a11fedf24b2dfb59a70c9e52b030f9a508dcd49cd2dd0d41956f4c785156b8`.

## Что действительно запускалось

Постоянный стенд [probe.py](probe.py) запускает скачанный CLI в bubblewrap:
исходное значение `HOME` сохраняется, но поверх этого пути монтируется
синтетический home. `CODEX_HOME` не задаётся. Окружение не передаёт токены,
реальные сессии и настройки не читаются; отдельное сетевое пространство без
внешней сети исключает обращения к API. Системный CLI не заменяется.

Проверены `--version`, `app-server --help`, JSON-RPC `initialize`, обнаружение
синтетического skill через `skills/list`, локальный `plugin/list`, а затем
3 прогревочных и 20 измеряемых циклов:

1. `thread/start` с `ephemeral: true`, без `turn/start`.
2. Новый thread присутствует в `thread/loaded/list`.
3. `thread/unsubscribe` возвращает `unsubscribed`.
4. Ожидание фактического исчезновения из loaded list.
5. Повторный unsubscribe возвращает `notLoaded`.

Во всех 23 циклах проверки прошли; были получены 23 `thread/closed`.
После EOF процесс завершился с кодом 0, дерево процессов стало пустым.
`thread_unload_delay_secs = 0` выбран для проверки освобождения; штатный
таймер 60 секунд этим опытом не проверялся. RPC-ответ unsubscribe сам по себе
не считается доказательством удаления runtime.

Память измерял существующий
[`proc_sampler.py`](../../../../Proteus-core/examples/performance/common/proc_sampler.py),
а не отдельная реализация чтения `/proc`. Сэмплы каждые 100 мс плюс точки
фаз; 153 наблюдения, без пропусков memory fields. Python-драйвер исключён,
вся измеряемая группа включает app-server и два небольших процесса bubblewrap.

| Точка | PSS дерева, KiB | RSS дерева, KiB | PSS app-server, KiB |
|---|---:|---:|---:|
| После прогрева и 3 с покоя | 139174 | 142780 | 138892 |
| После ещё 20 циклов и 3 с покоя | 145058 | 148664 | 144776 |
| После EOF | 0 | 0 | 0 |

Наблюдавшийся пик дерева: PSS **145342 KiB**, RSS **148948 KiB**;
SwapPss 0, наблюдавшийся CPU 0,30 с. После прогрева PSS увеличился на
5884 KiB. Последние пять точек после unload лежат в диапазоне
144942–145054 KiB, однако короткий ряд не доказывает отсутствие утечки.
Loaded runtimes освобождаются, а резидентная память сразу к исходной точке
не возвращается. Этот опыт не различает все внутренние кэши и allocator high
water и не проверяет реальные model turns, большие ответы tools, durable
resume или MCP-серверы. Абсолютную память нельзя напрямую сравнивать с
Proteus, запущенным на других входах.

Компактные исходные числа: [evidence-20261101.json](evidence-20261101.json).
Полные локальные отчёты: `build/reference/codex-internals-probe-20261101-ram/`.

## Что имеет смысл перенять, а что уже есть

### Явное освобождение runtime после остановки

Codex после `ShutdownComplete` явно отпускает conversation, config,
thread manager и turn-cost worker, оставляя лёгкий notification relay:
[`thread_lifecycle.rs:745`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/app-server/src/request_processors/thread_lifecycle.rs#L745).
Владение thread собственным event sender мешало дождаться закрытия канала:
это реальный класс циклического удержания, исправленный в
[PR #57788](https://github.com/openai/codex/pull/57788).

У Proteus уже есть аналогичный явный путь для UI-расширений:
[`close_context`](../../../../Proteus-desktop-extensions/sdk/src/host/scopes.rs#L154)
удаляет route и session state, а
[`Instances::prune`](../../../../Proteus-desktop-extensions/sdk/src/host/instances.rs#L25)
удаляет extension instance без живых session owners. Закрытие одного preview
при живом badge сессии закономерно не уничтожает instance. Проверять нужно
отдельно close(view), close_context, siblings и EOF, а не только закрытие окна.
Это сравнение границ владения, не доказательство одинакового backend lifecycle.

### Порционная проекция истории

Codex материализует JSONL в SQLite с курсором байтового смещения:
[`thread_history_materialization.rs:28`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/thread-store/src/local/thread_history_materialization.rs#L28)
ограничивает batch примерно 1 MiB исходника или 256 steps;
[`thread_history_materialization_pipeline.rs:14`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/thread-store/src/local/thread_history_materialization_pipeline.rs#L14)
оставляет в очереди два batch. Один большой record неделим, это не жёсткий
предел всей памяти процесса.

В Proteus уже существуют построчное чтение
[`session_journal/storage/read.rs`](../../../../Proteus-core/crates/proteus-core/src/core/session_journal/storage/read.rs),
проверяемый journal cursor и инкрементальная SQLite display projection
[`read_model/incremental.rs`](../../../../Proteus-core/crates/proteus-core/src/core/session_store/read_model/incremental.rs).
Повторно вводить SQLite или заменять canonical journal не требуется.
Полезен именно аудит промежуточных batch/очередей и максимального отдельного
record. Равенство пиков этих реализаций не измерялось.

### Ограничивать вывод до накопления полного результата

Codex использует
[`HeadTailBuffer`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/utils/head-tail-buffer/src/lib.rs)
с явным числом пропущенных байтов; unified exec удерживает до
[`1 MiB`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/core/src/unified_exec/mod.rs#L73).

Это уже есть в Proteus:
[`SESSION_BUFFER_LIMIT`](../../../../Proteus-modules/tool/shell-tool/src/unified_exec.rs#L25)
равен 1 MiB, а
[`ExecSession::push_output`](../../../../Proteus-modules/tool/shell-tool/src/unified_exec/session.rs#L160)
удаляет середину после каждого chunk и считает dropped bytes.
Отдельный disk capture имеет квоты и ограниченную очередь. Наличие ещё и
позднего обрезания model-visible результата не отменяет раннего buffer limit.
Добавлять второй такой буфер ради сходства с Codex не нужно.

### Совместное MCP-соединение — только при подходящем протоколе

Codex добавил process-owned
[`McpConnectionPool`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/codex-mcp/src/connection_pool.rs)
с weak registry и отдельными chat leases. Ключ учитывает настройки и auth
generation; legacy-соединения и серверы, которым нужен ввод при старте,
остаются отдельными. Требуется MCP 2026-07-28.
[`mcp_connection_sharing`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/features/src/lib.rs#L1654)
в этой версии **выключен по умолчанию**.

У Proteus
[`McpStdioHost`](../../../../Proteus-core/crates/proteus-core/src/tools/mcp/client.rs)
уже поддерживает постоянное соединение, совместное для tools одного host;
[`register_mcp_tools`](../../../../Proteus-core/crates/proteus-core/src/tools/mcp.rs#L97)
создаёт host при регистрации server. Это не доказывает, что каждый чат
обязательно запускает новый MCP-процесс: дополнительно важен срок жизни
общего ToolRegistry. Cross-registry pooling — исследовательский кандидат,
не подтверждённая утечка или безопасная настройка для немедленного включения.

### Каталоги skills/plugins: метаданные отдельно от тела

Codex имеет бюджетируемые
[`skill catalog deltas`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/ext/skills/src/catalog_delta.rs),
но [`skill_catalog_deltas`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/features/src/lib.rs#L1930)
**выключен по умолчанию**. Plugin catalog может отвергать устаревший cache,
прочитав только 256-byte header перед полной десериализацией:
[`catalog_cache.rs:159`](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/core-plugins/src/remote/catalog_cache.rs#L159).
`plugin_discovery_prefetch` штатно включён, но отключаем. В нашем offline
опыте remote plugins и prefetch отключены, поэтому сетевой каталог не проверен.

Proteus уже выдаёт метаданные skills с `BodyMode::Omit`, а расширения могут
ссылаться на инструкцию через `agent_instructions`. Это не означает
автоматическую установку или знание агента об инструкции. Найдено конкретное
место для дальнейшей экономии:
[`invoke_skill`](../../../../Proteus-modules/tool/skill-pack/src/lib.rs#L164)
сначала делает `discover_skills(..., BodyMode::Include)` для всех skills,
затем выбирает один. Таким образом, тела остальных skills временно
удерживаются без необходимости: до порядка 32 MiB при 128 файлах по 256 KiB.
Это верхняя оценка по лимитам исходников, не измеренный пик.
Будущее selective-body чтение должно сохранить нынешнюю полную валидацию
всех файлов, приоритеты user/project/plugin и ошибки discovery; простой
непроверенный повторный read выбранного пути изменит эти гарантии.

Сопоставлялись исходники Proteus: Core `781a47a`, Modules `1dd841d`,
Desktop `774318b`, Extensions `1fa4351`. Production-код этим исследованием
не менялся. Более поздние исправления могут закрыть перечисленные кандидаты.

## Повторить

Скачанный бинарник хранится отдельно от системного в
`build/reference/codex-binary-rust-v0.166.0/`. Из корня workspace:

```sh
python3 Proteus-desktop/examples/research/codex-internals/probe.py \
  --binary Proteus-desktop/build/reference/codex-binary-rust-v0.166.0/codex-x86_64-unknown-linux-musl \
  --output Proteus-desktop/build/reference/codex-internals-probe-new \
  --sampler Proteus-core/examples/performance/common/proc_sampler.py \
  --warmup 3 --cycles 20 --idle 3
```

Выходной каталог должен быть новым. Не запускайте одновременно с другим
ресурсным замером или сборкой. `--sampler` можно опустить для проверки одного
протокола без численных утверждений о памяти.
