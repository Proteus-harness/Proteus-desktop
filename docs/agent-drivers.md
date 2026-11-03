# Один интерфейс для нескольких агентов

Состояние на 2026-11-03. Основной Desktop подключает Proteus через HTTP/SSE.
Эксперимент прямого подключения внешнего Codex через
`codex app-server --listen stdio://` отложен владельцем из-за проблем при
использовании. Текущий приоритет — обычный Proteus backend, удаление сессий
и завершение tool-команд. Эксперимент возобновляется по отдельному решению;
сохранённые ниже описание и код не означают готовности к повседневной работе.
Codex-compatible model/tool adapters внутри Proteus этим решением не
откладываются. Claude Code и ACP client также остаются отложенными;
реализация терминального движка описана ниже отдельно.

## Codex: сохранённый эксперимент

Нужен отдельно установленный `codex` с app-server и уже настроенным входом.
Desktop использует обычные конфигурацию и авторизацию Codex; ключи из профилей
Proteus в него не переносит. Путь к executable задаётся через
`PROTEUS_CODEX_EXECUTABLE`, по умолчанию используется `codex` из `PATH`:

```bash
./build/Proteus/proteus-native /path/to/project --agent codex
PROTEUS_CODEX_EXECUTABLE=/path/to/codex ./build/Proteus/proteus-native /path/to/project --agent codex
```

Выбор агента для новых чатов сохраняется. `--agent proteus` или `--agent codex`
задаёт его явно при запуске; сохранённые вкладки восстанавливают своего агента.
`--origin` относится только к Proteus и несовместим с `--agent codex`.

В общей ленте Codex доступны поток текста и tools, состояние работы, остановка
хода, восстановление thread, запросы разрешений и вопросы пользователю.
Модели и effort берутся из каталога Codex. История читается страницами по 50
элементов через непрозрачные курсоры; sidebar использует только summary.
При первом запуске без сохранённого чата создаётся свой новый thread,
случайная последняя сессия другого клиента не выбирается.

Один принадлежащий Desktop процесс app-server обслуживает открытые чаты
и отделённые окна. Скрытые вкладки не загружают историю до открытия;
Codex-only workspace не запускает Core и его модули. Закрытие вкладки снимает
подписку, кнопка остановки прерывает ход, а завершение Desktop закрывает
принадлежащий ему процесс. Ошибки и EOF не повторяют отправленный prompt.
Очереди транспорта ограничены по байтам и числу сообщений; это не обещание
фиксированной памяти самого Codex или его текущего хода.

Удаление сначала снимает подписку открытого thread. Подтверждённый отказ
Codex восстанавливает ту же подписку, текущий ход и ограниченный хвост истории,
сохраняя черновик и позицию чтения. Timeout оставляет результат неопределённым:
Desktop не повторяет удаление и не возобновляет thread автоматически.
Уведомления `thread/deleted` обрабатываются независимо от открытого чата,
включая удалённых потомков и вкладки в других окнах. Отсутствие чата на первой
странице каталога не считается доказательством удаления.

Изменения выбранной вкладки и сессии применяются сразу в памяти. Настройки
сохраняет один фоновый writer: текущая запись и последний ожидающий снимок,
без очереди всех кликов и без обновления всех окон при изменении метаданных.

В текущем MVP управление профилем, modules/hooks/policy и другие Core-настройки
не применяются к Codex. Связанные с Core панели файлов и Git для него
недоступны; терминал работает с рабочей папкой без Core через общую декларацию
`dependencies.core: false`. Контентные рендереры используются общие. Присоединение к
терминалу, созданному самим агентом, ещё не реализовано.

Код: `crates/codex-client`, `src/agents/codex`, `src/ui/chat/codex`.
Синтетический app-server без аккаунта и модели описан в
[`examples/codex/README.md`](../examples/codex/README.md). Тесты транспорта,
проекций, восстановления и UI-состояний находятся рядом с реализацией;
это отдельно от проверки реального аккаунта и нативного окна.

Общей является граница driver, а не обязательный wire protocol. ACP остаётся
возможным способом подключения следующих агентов. Ниже — исследование
дальнейших границ, выполненное 2026-11-01.

## Решение

Сохранить чат центром приложения. Проект задаёт рабочие файлы и Git;
чат выбирает исполняющего агента. Документы, терминалы и изменения остаются
связанными рабочими объектами, которые можно открыть рядом с чатом.
Proteus, Codex и Claude Code — разные исполнители, а не провайдеры модели.
Список моделей и параметры доступа приходят от выбранного исполнителя.
Настройки modules/hooks/policy Proteus не показываются как настройки Claude
или Codex.

Нужны три независимые границы:

1. `AgentSession` — подключение агента и жизненный цикл разговора.
2. `TerminalSession` — процесс, PTY, ввод, размер и ограниченный поток вывода.
3. VT engine — разбор escape sequences и состояние экрана.

Замена третьей границы не создаёт первые две.

## ACP в существующем Desktop

Core уже предоставляет `server acp` для внешних редакторов. Это agent-side;
готового Desktop ACP client из него не получается.

Ветка Proteus в `ui/chat` сохраняет Core `Api`, SSE pump и Core DTO.
Codex использует отдельные typed events и непрозрачный thread ID, которые
проецируются в общую ленту без выдуманных Core UUID или session directories.
`ChatKey` различает `Proteus(PathBuf)` и `Codex(String)`; summary не требует
загрузки transcript. Общий контракт следующего driver должен сохранять эти
границы:

| Операция | Общая семантика |
|---|---|
| Create/load/resume | Непрозрачный ID внешней сессии + стабильный backend ID |
| Prompt/cancel | Один ход, его состояние и завершение |
| Events | Текст, tools, результат, план, запрос разрешения |
| Capabilities | Поддерживаемые модели, режимы, настройки, история |
| Lifecycle | Ошибка/EOF процесса, reconnect, shutdown без повторного prompt |

Следующий ACP driver следует строить через готовый Rust SDK и отдельную
проекцию событий, сохраняя общие renderer, viewport и клавиатуру. Локальный
архив сам по себе не восстанавливает контекст внешнего агента; восстановление
должно подтверждаться его транспортом.

Ориентир в локальном Zed `9698422` — `crates/acp_thread/src/connection.rs`,
`AgentConnection`: native и ACP соединения скрыты за общей границей,
модели/режимы/config options доступны по capabilities.

### Состояние внешних компонентов

- В официальной документации ACP v1 — Latest, v2 — Draft. Номер версии SDK
  не равен автоматически wire protocol version.
- Актуальный [codex-acp v2.5.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.5.0)
  — TypeScript adapter поверх Codex App Server. Это отдельный проект ACP,
  не встроенный ACP transport OpenAI Codex. Старый Zed fork архивирован.
- [claude-agent-acp v0.99.1](https://github.com/agentclientprotocol/claude-agent-acp/releases/tag/v0.99.1)
  использует Claude Agent SDK и Node. Нативный интерфейс приложения не
  устраняет память внешнего SDK/adapter process.
- Оба адаптера реализуют создание/загрузку/возобновление/список/закрытие,
  tools, permissions, cancel и config options; поддержку согласуем при
  initialize. Расширенные subagent/steering/questions/background events
  могут зависеть от draft или частных opt-in extensions.

Источники: [Rust SDK](https://agentclientprotocol.com/libraries/rust),
[config options](https://agentclientprotocol.com/protocol/v1/session-config-options),
[Codex adapter source](https://github.com/agentclientprotocol/codex-acp/blob/v2.5.0/src/CodexAcpServer.ts),
[Claude adapter source](https://github.com/agentclientprotocol/claude-agent-acp/blob/v0.99.1/src/acp-agent.ts).

Для Codex выбран [официальный App Server](https://developers.openai.com/codex/app-server):
создание/восстановление threads, turns, streaming и approval requests доступны
напрямую. Дополнительный Node/ACP adapter в текущем подключении не запускается.
Второй Codex driver через ACP до измерений не нужен.

## Открыть терминал работающего tool

Сегодня Desktop `terminal::Terminal::spawn` запускает shell и владеет своим
PTY; shell-tool `unified_exec::spawn_pty` в другом процессе владеет другим
master. У module `ProcessControl` есть session/thread/cwd owner, но нет resize.
Сетка фиксирована 80×24, `wait_and_collect` потребляет общий буфер через
`mem::take`. Второй независимый читатель украл бы вывод у агента.

Нужен один владелец PTY с устойчивым terminal ID:

- один последовательный поток, ограниченный по байтам, с независимыми
  курсорами tool и UI, а не две копии полного scrollback;
- input/resize/exit и проверка владельца;
- отдельные действия «закрыть вкладку» и «остановить процесс»;
- кнопка у tool открывает тот же ID и процесс, без запуска второго shell;
- при переполнении медленный зритель получает явный gap/snapshot, а tool
  сохраняет оговорённую семантику вывода.

Для `tty=false` можно показать поток вывода. Превратить pipes в интерактивный
TUI задним числом или произвольно присоединиться по PID нельзя обещать.
Терминал, изначально созданный общим broker, можно показывать во вкладке.

[ACP v1 terminal methods](https://agentclientprotocol.com/protocol/v1/terminals)
дают create/output/wait/kill/release и terminal ID, но не полноценный
интерактивный input/resize. В draft v2 клиентские terminal methods убираются;
[agent-owned terminal display](https://agentclientprotocol.com/protocol/v2/tool-calls)
также не предоставляет интерактивность. Поэтому broker должен жить отдельно
от версии ACP. Локальный Zed `acp_thread/src/terminal.rs::create_terminal_entity`
даже закрывает stdin через `/dev/null`; его нельзя копировать как готовый
interactive attach.

## Терминальный движок Ghostty

Используется `libghostty-vt`: VT parser/state, scrollback/reflow,
input encoders и render state. По [релизу 1.4.0](https://ghostty.org/docs/install/release-notes/1-4-0)
от 2026-10-27 уже есть полный C API, но API/ABI ещё меняются и отдельного
tagged libghostty release пока нет. Официальная граница C/Zig; для Rust нужны
bindings. Это не готовые GPUI renderer, PTY или session manager — см.
[минимальный consumer Ghostling](https://github.com/ghostty-org/ghostling#what-is-libghostty).

Движок выделен в `crates/terminal`; `src/terminal` сохраняет общий API для UI.
Ghostty подключён через `ghostty-vt 0.8.0`, закреплённый на Ghostty `83fb956`.
Rust bindings сохранены в `vendor/ghostty-vt*` с исходными лицензиями; проверенный
MSRV снижен до используемого Rust 1.95. Изменения относительно upstream описаны
в `vendor/ghostty-vt*/PROTEUS.md`. Парсер Ghostty не переписан.

Один actor владеет VT и PTY (`portable-pty`), кодирует клавиши согласно режимам
терминала и публикует только последний видимый `Screen`. В UI не выполняются
разбор вывода и reflow истории. Очередь пользовательского ввода ограничена
1 МиБ, ответов PTY — 64 КиБ; слишком большая вставка отклоняется без закрытия
оболочки. Скроллбек ограничен примерно 8 МиБ и 10 000 строк, с погрешностью
целых страниц Ghostty; это не предел полного RSS процесса. Невидимые страницы
постепенно сжимаются во время простоя. Второй emulator или полный raw replay
для одного терминала не создаётся.

На Linux завершение терминала учитывает всю его PTY-сессию, включая уже
переподчинённые фоновые jobs; сигналы адресуются через `pidfd`. После выхода
из цикла приложения общий shutdown ждёт освобождения принадлежащих ему PTY
до двух секунд. Обычное закрытие вкладки не ждёт cleanup в потоке интерфейса.

Feature `ghostty-vt` включает нативную библиотеку; переменная
`PROTEUS_TERMINAL_ENGINE=ghostty|alacritty` выбирает движок для сравнений.
В обычной сборке feature включена и по умолчанию выбран Ghostty;
`--no-default-features` оставляет прежний Alacritty без Zig-зависимости.
Сборочные инструменты готовит `scripts/terminal_toolchain.py`; Zig и исходные
архивы сохраняются в build-кэше и не попадают в runtime-поставку. Команды —
в [`BUILDING.md`](../BUILDING.md).

Заявленные upstream 70–90% экономии resident memory относятся к их собственному
сжатию невидимого text-heavy scrollback; это не сравнение с нашим Alacritty.
Проверка — по A/B replay на одинаковых release-сборках: ASCII/Unicode, ANSI,
полноэкранный TUI, resize/reflow, 10k/100k строк, холодный/повторный запуск,
PSS/RSS/peak/CPU и latency. Сначала один engine и отсутствие регрессий, затем
сравнение. Воспроизводимый стенд — `crates/terminal/examples/replay.rs`;
каждый движок и размер запускаются отдельным процессом одного release binary,
с измерением PSS/RSS сразу после вывода и после секунды простоя.

## Что показал Zeron

Проверен [v0.2.131 / 0a153df](https://github.com/zeronsh/zeron/tree/0a153df5ad84a2036d327bd785281888689f4030),
исходники сохранены в `build/research/zeron`. Установки и запуска не было.

Их [harness](https://github.com/zeronsh/zeron/blob/0a153df5ad84a2036d327bd785281888689f4030/crates/harness/src/lib.rs)
использует Codex native app-server, Claude CLI stream-json, Cursor SDK shim,
OpenCode HTTP/SSE и ACP для native ACP агентов. Adapter-mediated ACP для
Claude/Codex/Cursor ранее убрали из-за lifecycle/background-task проблем.
Это их конкретный опыт; он не доказывает сбой нынешних версий адаптеров.
Прямой Claude driver тоже имеет цену: использует vendor wire и флаг, который
проект сам помечает undocumented.

В актуальном [UI коде](https://github.com/zeronsh/zeron/blob/0a153df5ad84a2036d327bd785281888689f4030/crates/ui/src/shell/tabs.rs)
полоса главных session tabs уже удалена: sidebar выбирает чат, titlebar
его показывает, справа открываются связанные file/browser/diff/terminal/
subagent/side-chat/PR surfaces. Правая раскладка принадлежит выбранному чату.
Старые ARCHITECTURE/research документы местами ещё описывают прежний вариант.

Для Proteus это сильный ориентир следующей итерации: главная навигация по
чатам, документы рядом. Удаление нынешних chat tabs, splits и detach либо
введение отдельной раскладки каждого чата требуют самостоятельного решения;
текущая правка sidebar этого не делает.

Их [terminal service](https://github.com/zeronsh/zeron/blob/0a153df5ad84a2036d327bd785281888689f4030/crates/engine/src/terminals.rs)
владеет PTY в engine, выдаёт terminal ID и sequence/replay, принимает ввод
и resize; UI можно отключить без остановки процесса. Используют
portable-pty 0.8 и alacritty_terminal 0.26, не Ghostty. Ограничения — до 32
терминалов, replay 1 МиБ/терминал, UI scrollback 10k строк. Однако raw-output
и subscriber queues местами unbounded: одни лимиты replay не доказывают
ограниченную память всего пути.

Лицензия MIT разрешает адаптировать отдельные реализации с notices. Стоит
перенять driver boundary, различие stop-turn/destroy-runtime, engine-owned
terminal и lifecycle fixtures. Переносить целиком persistence/sync и все
UI surfaces ради этой задачи не требуется.

## Порядок и сложность

| Этап | Размер изменения | Проверяемый результат |
|---|---|---|
| Прямой Codex в Desktop | Эксперимент отложен | код общего чата, opaque IDs, streaming, permissions, cancel, restore и pagination сохранён |
| Следующий агент через ACP | Отложено | отдельный transport и capabilities без Core-настроек |
| Общий PTY broker + вкладка того же процесса | Самостоятельный крупный | интерактивный ввод/resize, tool не теряет вывод, close ≠ kill |
| Ghostty A/B | Ограниченный эксперимент | ресурсные метрики и совместимость, решение о замене |

При возобновлении экспериментов сначала нужен scripted peer без модели и реального
аккаунта, затем проверка настоящим агентом с выбранным профилем. Ресурсные
ворота: никакого eager запуска процессов для всех чатов, ограниченные очереди,
без полной истории ради списка, учёт PSS всего дерева процессов. Точные сроки
до этого прототипа ненадёжны. Все три подсистемы одним рефакторингом не менять.
