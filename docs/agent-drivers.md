# Один интерфейс для нескольких агентов

Исследование на 2026-11-01. Это предложение границ и порядка работ;
ACP client, общий PTY broker и Ghostty ещё не реализованы.

Итог после проверки Zeron: общей должна быть граница driver, а не обязательный
wire protocol. ACP — один способ подключения; native transport остаётся
допустимым там, где он лучше передаёт lifecycle агента.

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

Сейчас `ui/chat` напрямую использует Core `Api`, SSE pump, `SendRequest`,
`AppSessionSnapshot`, `ConfigSummary`, UUID и `PathBuf session_dir`.
`transcript::Entry` близок к независимому представлению: стабильный ключ,
`Arc<str>`, streaming, время. Tools/subagents пока тоже используют Core DTO.

Минимальный новый слой между UI и транспортом:

| Операция | Общая семантика |
|---|---|
| Create/load/resume | Непрозрачный ID внешней сессии + стабильный backend ID |
| Prompt/cancel | Один ход, его состояние и завершение |
| Events | Текст, tools, результат, план, запрос разрешения |
| Capabilities | Поддерживаемые модели, режимы, настройки, история |
| Lifecycle | Ошибка/EOF процесса, reconnect, shutdown без повторного prompt |

Два driver: нынешний Proteus HTTP/SSE и ACP stdio через готовый Rust SDK.
UI проецирует оба в небольшую модель ленты; ACP не следует выдавать за
выдуманные runtime events Proteus. Renderer, виртуализация, оптимистичная
отправка, viewport и клавиатура остаются общими.

Сохраняем `{backend_id, external_session_id, workspace}`. Локальный архив
даёт просмотр чата, но сам по себе не восстанавливает контекст внешнего
агента. В sidebar достаточно компактных summary; скрытые чаты не должны
держать отдельный процесс агента или загружать всю историю.

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

Для Codex есть и [официальный App Server](https://developers.openai.com/codex/app-server):
создание/восстановление threads, turns, streaming, approval requests. Если
ACP adapter окажется неприемлем по памяти или потеряет нужную возможность,
тот же `AgentSession` допускает отдельный native driver. Одновременно
реализовывать оба Codex driver до измерений не нужно.

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

## Эксперимент с Ghostty

Практический кандидат — `libghostty-vt`: VT parser/state, scrollback/reflow,
input encoders и render state. По [релизу 1.4.0](https://ghostty.org/docs/install/release-notes/1-4-0)
от 2026-10-27 уже есть полный C API, но API/ABI ещё меняются и отдельного
tagged libghostty release пока нет. Официальная граница C/Zig; для Rust нужны
bindings. Это не готовые GPUI renderer, PTY или session manager — см.
[минимальный consumer Ghostling](https://github.com/ghostty-org/ghostling#what-is-libghostty).

У нас слой замены — `src/terminal`: сейчас alacritty_terminal 0.25 одновременно
даёт emulator и PTY event loop, поэтому при замене надо явно сохранить I/O.
`Screen` snapshots и GPUI view можно оставить как внешнюю границу.

Заявленные upstream 70–90% экономии resident memory относятся к их собственному
сжатию невидимого text-heavy scrollback; это не сравнение с нашим Alacritty.
Решение — по A/B replay на одинаковых release-сборках: ASCII/Unicode, ANSI,
полноэкранный TUI, resize/reflow, 10k/100k строк, холодный/повторный запуск,
PSS/RSS/peak/CPU и latency. Сначала один engine и отсутствие регрессий, затем
сравнение. Не держать два emulator/полных буфера в обычном режиме.

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
| Разделить agent/model и выделить driver | Средний | Proteus работает через новую границу без смены поведения |
| ACP MVP одного внешнего агента | Средний | prompt, streaming, tools, permissions, cancel, EOF |
| История/restore и второй агент | Крупный | разные capabilities, opaque IDs, restart и pagination |
| Общий PTY broker + вкладка того же процесса | Самостоятельный крупный | интерактивный ввод/resize, tool не теряет вывод, close ≠ kill |
| Ghostty A/B | Ограниченный эксперимент | ресурсные метрики и совместимость, решение о замене |

Первый прототип проверяется scripted ACP peer без модели и реального аккаунта,
затем настоящим адаптером с явно выбранным пользователем профилем. Ресурсные
ворота: никакого eager запуска процессов для всех чатов, ограниченные очереди,
без полной истории ради списка, учёт PSS всего дерева процессов. Точные сроки
до этого прототипа ненадёжны. Все три подсистемы одним рефакторингом не менять.
