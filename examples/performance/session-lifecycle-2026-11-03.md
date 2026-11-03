# Удаление сессий и завершение команд — 2026-11-03

Исправлены конкретные дефекты жизненного цикла в Desktop, Core и shell-tool.
Они объясняют возможную потерю результата удаления, ожидание чужой сессии
и зависание при завершении pipe-команд. Точная команда из пользовательского
случая с числом `111` пока неизвестна; совпадений в проверенных реальных
журналах не найдено. Синтетические регрессии не доказывают, что установлен
именно тот же источник ошибки.

## Что было исправлено

### 1. Закрытие открытой сессии принималось за остановку всего backend

Desktop получал `AppServerEvent::Shutdown` в адресном stream одной сессии
и передавал его как app-wide `ServerStopped`. Это могло завершать общий Core
и лишать запрос удаления его ACK. Теперь Shutdown закрывает только поток
и pending-запросы этой сессии. Общую Core lease завершает принадлежащий
приложению process watcher, а не сообщение отдельного чата.

При потере соединения интерфейс приостанавливает анимацию незавершённых tools,
цепочек и индикатора `ChatList.running`; показывает проблему связи, а не
подтверждённую продолжающуюся работу. Канонические run/tool outcomes при этом
не переписываются на success, canceled или failed. После переподключения
снова используются подтверждённые сервером состояния, включая работу peers.

### 2. Долгое удаление одной сессии блокировало остальные

Core удерживал один глобальный lifecycle mutex через ожидание settlement
удаляемой сессии. Поэтому создание, resume или удаление другой сессии также
могло ждать эту работу. Замена — отдельная gate по каноническому пути сессии;
путь каталога и её `journal.jsonl` получают одну identity, даже если каталог
ещё не создан. Registry хранит weak references, а gate живёт у владельцев
и ожидающих операций.

Принятое удаление исполняется в принадлежащей серверу задаче и переживает
отмену HTTP waiter. Она держит path guard через settlement, снятие runtime
из registry и физическое удаление данных. Resume того же пути ждёт завершения
и не возвращает закрывающуюся generation; другие сессии остаются доступны.
Это не отменяет необходимое ожидание собственной активной работы и не
является обещанием мгновенного удаления при медленном storage/runtime.

### 3. Pipe-потомки могли не дать команде завершиться

У `exec_command` с pipes выход лидера больше не освобождает PID/PGID до
окончания drain. `WNOWAIT` сохраняет identity; оставшаяся process group
останавливается до reap. После reap либо потери child ownership сохранённый
handle не получает права послать сигнал переиспользованному PID.

One-shot `shell` читает stdout/stderr неблокирующе. После timeout, cancel
или ошибки cancellation channel reader threads получают короткий финальный
drain, закрывают свои fd и присоединяются. Поэтому даже потомок, покинувший
group через `setsid` и сохранивший inherited pipes, не продлевает join бесконечно.
Его собственное принудительное завершение этим не гарантируется.

При review первоначальный периодический опрос новых readers каждые 5 мс
был признан CPU-регрессией и заменён ожиданием readiness через `poll` и owner-only
wake fd. Одна stop-нотификация будит обоих readers; исчезновение owner также
будит их через HUP. Это относится к новым output readers, а не к утверждению,
что весь process lifecycle теперь работает совсем без polling.

## Граница случая с `111`

- Число `111` само по себе не определяет, был ли это exit code команды,
  OS error или текст вложенного инструмента. Точная команда неизвестна.
- В тестах воспроизводится обычный ненулевой **exit code 111** с inherited
  output: сохраняются exit code, хвост stdout/stderr и завершение capture.
  Специальной lifecycle-семантики для этого числа нет.
- В Desktop тест потери связи использует строку `os error 111` как синтетическую
  ошибку transport. Это проверяет паузу индикаторов и сохранение canonical
  state, а не устанавливает причину реального пользовательского случая.
- Обычный one-shot `shell` после выхода лидера по-прежнему ждёт EOF потомков
  до command timeout; default — **600 секунд**. Изменение ограничивает teardown
  после timeout/cancel/error, но не обещает немедленный возврат для любой
  команды, оставившей pipes открытыми.

## Проверка

| Scope | Результат | Evidence |
|---|---|---|
| Core HTTP app-server | 81 passed, 0 failed | `core-tests.log` |
| Desktop workspace | 598 passed, 2 ignored, 0 failed | `desktop-tests-final.log` |
| Modules `shell-tool` | 109 passed, 0 failed | `modules-tests-final.log` |

Core regression удерживает settlement сессии A, отменяет её HTTP waiter,
проверяет ожидание resume того же canonical path и независимые операции B;
после settlement A удаляется без воскрешения storage. Desktop entity regression
проверяет локальность Shutdown и сохранение shared API identity; отдельные
проверки покрывают disconnected tool status и process-watcher scope.

Shell regressions покрывают nonzero exit с inherited pipes, capture, timeout,
cancel и ошибку cancellation channel при detached потомке, отсутствие
периодических wakeup тихого reader и пробуждение обоих readers по stop/HUP.
Оба пропущенных Desktop теста — явные installed Codex app-server smoke
и extension conformance; их прохождение этим прогоном не заявляется.

Измеренного выигрыша RSS/PSS, общего CPU приложения или latency удаления
пока нет. Устранение глобального ожидания и конкретных зависших joins
подтверждено поведением синтетических regression tests.

## Код и согласованное отложенное направление

- Core `fac4ca4d6de83a5b16a34061fdd42fbafe18ded8`;
- Modules `339456df120a7f5816afb1169384c48717f7957f`;
- Desktop `488311e4672c66a4aeec97e048dd51173a85bf76`.

Эксперимент внешнего Codex в Desktop отложен владельцем. Это записано в
README и `docs/agent-drivers.md`; код не вычищен, действующие Codex-compatible
model/tool adapters Proteus не отключены. Текущий приоритет — обычный backend,
удаление сессий и завершение команд.

## Сборка и provenance

Обычный release, `--components desktop core modules`, два build jobs, caches
сохранены. Расширения переиспользованы. Первый проход компилирует изменения
с уже готовыми зависимостями; это не холодная сборка и не сравнение с прежней
версией. Оба прохода завершились с exit **0**, sampling завершён полностью.
Native-проверка началась после окончания сборки.

| Показатель | Изменённые компоненты | Повтор без изменений |
|---|---:|---:|
| Wall | 528,88 с | 3,97 с |
| user + system CPU | 876,89 с | 3,36 с |
| Напечатано `Compiling` packages | 4 | 0 |
| Sampled peak PSS дерева | 2464,86 МиБ | 150,31 МиБ |
| Sampled peak RSS дерева | 2481,54 МиБ | 164,36 МиБ |
| Максимальный RSS отдельного процесса | 2331,76 МиБ | 151,48 МиБ |

Фазы Cargo: Desktop **5:42**, Core **2:56**, Modules **6,30 с**. Повтор:
**0,65 / 0,12 / 0,15 с**. В Modules заново компилировались только `shell-tool`
и `proteus-shell-tool-module`; новые dependencies не добавлялись. Sampled peak
может пропустить короткий пик. RSS дерева повторно учитывает общие страницы;
пик одного процесса не складывается с показателем дерева. Это память сборки,
не расход работающего приложения.

Cargo resolve для Linux (с dev/build dependencies) остался тем же:
Desktop — 6 workspace packages / 681 resolved / 2005 рёбер;
Core — 5 / 218 / 510; Modules — 38 / 325 / 885. Это не число свежих компиляций;
графы отдельных workspace нельзя складывать как уникальные dependencies.

Проверены actual SHA256 всех **36** файлов release manifest. Изменились
**только 3** — Native, Core и shell module; остальные **33**, включая все
14 extension executables, совпали с готовой поставкой до обновления.

| Файл | SHA256 |
|---|---|
| `bin/proteus-native` | `576452a34b4526cf97bf407d22831a5dc30581c369b3ada09c3c822633b25f30` |
| `bin/proteus` | `5508cd8c8d8125e452876976b774610ed321524a840493f1fb2b93f4f8bcb512` |
| `bin/proteus-shell-tool-module` | `831a7f55ab07b165a66112dec4055fe1b50c16c45a3f7850dac74221a630972c` |

Sources готовой поставки — перечисленные выше code commits, `dirty: false`.
Extensions сохранены из `7b645271f9cd1d164ff92f06a287532d2ca122a3`.
Документационный commit после code commit не требует новой компиляции.

## Native-проверка

Запущен Native из готовой `build/Proteus/bin` с принадлежащим ему Core из
той же папки, в отдельном background-gui сеансе. Профиль взят из существующего
`native-history/run.py::profile`, две истории по **20 сообщений** подготовлены
готовым `session_index_fixture` и проверены `history-scale/verify.py`. Все
config/data/XDG directories частные. Локальная fake-модель исключает внешнего
провайдера; extensions в этом функциональном запуске не загружались.

1. Оба чата проверены через authenticated HTTP/SSE: session ID, 20 сообщений,
   тексты и IDs хвоста совпали с fixture.
2. Открытый чат удалён через контекстное меню и подтверждение в Native.
   Его вкладка и строка исчезли, каталог удалён; HTTP `/sessions` содержит
   только другой чат. Это подтверждает, что ACK применился в клиенте.
3. Общий Core сохранил PID **49065** и `start_ticks` **747672**.
   История второго чата после удаления всё ещё содержит исходные 20 сообщений.
4. Второй чат открыт в Native, через composer отправлен
   `SESSION-LIFECYCLE-NATIVE-OK`. В окне появился ответ fake-модели; свежий
   SSE snapshot подтвердил **22 сообщения**, исходный prompt и final answer.
   Core сохранил ту же identity.
5. Окно закрыто штатной кнопкой. Проверка PID + start_ticks не нашла ни одного
   живого процесса из **10** наблюдавшихся Native/Core/module процессов.
   После этого private GUI-сеанс остановлен; compositor/observer не считаются
   процессами приложения.

Измерения click-to-present latency нет: промежуточный screenshot может
предшествовать применению ACK. Приняты settled кадры с исчезнувшим чатом
и ответом во втором. Эта малая синтетическая проверка не доказывает latency
на огромных историях или результат реального неизвестного случая с `111`.

Локальный evidence-каталог: `build/verification/session-lifecycle-20261103/`:
final test logs, `build-cost/report.json`, `ready-before.json`, `ready-after.json`,
`artifact-checks.json`; `native-delete/` содержит seed/expected receipts,
`before-checks.json`, `native-checks.json`, `close-checks.json`, process snapshots
и PNGs. Первоначальный compile-only сбой пути test module сохранён отдельно;
финальные gates относятся к исправленной версии. Evidence не содержит
пользовательских чатов.
