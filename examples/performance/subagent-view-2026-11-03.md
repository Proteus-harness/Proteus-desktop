# Просмотр чатов субагентов: 3 ноября 2026

Субагент представлен компактной строкой без вложенной карточки инструментов.
Клик открывает сохранённый чат справа; повторный клик выбирает ту же вкладку.
Просмотр использует общий Chat, History и TabLayout, без composer и второго
writer/runtime ребёнка. История читается через существующий GET `/history/page`.

## Исправленная граница

`child_thread_id` обозначает логическую задачу, а не UUID durable session.
Прежний fresh reuse очищал историю в той же session. Теперь новая задача
получает отдельную session в прежнем OS-процессе; resume сохраняет свою session.
Старый bridge остановлен и joined до shutdown, поэтому его Shutdown не попадает
в следующий ход. Это не обещание сохранения прогретых model clients при rotation.

После первого durable TurnOpened публикуется точная привязка UUID + directory.
Компактный immutable `SubagentSessionBoundRecorded` связывает её с исходным
вызовом `task`, `spawn_agent` или idle `followup_task`. Поздняя запись разрешена
после завершения родительского хода, с проверкой владельца и разрешённого вызова.
Конфликтующая или повторная привязка отклоняется. Журнал остаётся schema 21;
disposable SQLite index обновлён до format 3 и при необходимости перестраивается.

Очередь обновления completed view сохраняет pending во время settlement,
объединяет запросы и отклоняет результат, который опередил новый активный ход.
Сохранённая строка и её живое состояние показывают один визуальный переход.
Старые задачи без сохранённой привязки остаются недоступными с пояснением.

## Проверки

| Проверка | Результат |
|---|---:|
| Core: `python3 scripts/test.py full` | 1079 PASS, 34 targets, exit 0 |
| Core: `module_swap` с обновлёнными consumers | 17 PASS |
| Modules: `python3 scripts/test.py full` | 632 Rust PASS, 13 Python PASS, 33 Node PASS |
| Desktop: `python3 scripts/test.py --offline -j 2` | 611 PASS, 2 ignored, exit 0 |
| `native-stream` Python suite | 23 PASS |

Проверены stdio resume, fresh reuse с сохранением прежних байтов журнала,
блокировка rotation при активном запросе, deadline/cancel bootstrap,
отказ записи привязки, cold JSONL/SQLite projection, точная source attribution,
поздняя привязка, stale responses и сохранение родительского command context.

Native проверен вручную на готовом release launcher в отдельном background-gui
сеансе `20261103-125244-4c576bfe55`, 1600×900. У двух детей одинаковый role и
разные UUID/каталоги; каждый содержит 150 сообщений. Подтверждено:

- до клика нет чтений детей;
- повторный клик не создаёт новую панель и не читает историю повторно;
- A и B показывают свои истории вплоть до row 000, страницами по 50;
- UNBOUND не открывает другую историю;
- завершение A добавляет видимый финальный ответ одним tail-запросом;
- ручное «Обновить» добавляет ровно один запрос той же revision;
- закрытие панелей сохраняет основной чат и его единственное подключение.

Финальные счётчики: A — 5 чтений (3 tail, 2 older), B — 3 (1 tail, 2 older).
Child resume/SSE — 0; parent resume/SSE — по 1. Ошибок fixture нет.
На снимке `11-child-a-final-tail.png` видны основной чат, его composer и
сохранённый финальный ответ справа без дочернего composer. Native и отдельный
display остановлены, PID Native завершён.

Локальное evidence: `build/verification/subagent-panel-native-20261103/`
(`native-check.json`, `state.json`, `trace.json`, screenshots). Автоматическое
`state.native_verified=false` само не доказывает GUI; отдельный manual receipt
содержит проверенные изображения и SHA256 исполняемого ELF.

## Сборка

Обычный release, jobs=2, компоненты `desktop core modules`, без архива.
Cargo/OS caches сохранены. Второй прогон без изменений исходников.

| Показатель | После изменения contracts | Повтор |
|---|---:|---:|
| Полное время | 702,86 с | 4,11 с |
| CPU user + system | 1204,30 с | 3,49 с |
| Наблюдаемый пик PSS дерева сборки | 2,417 GiB | 0,160 GiB |
| Строки Cargo `Compiling` | 48 | 0 |

Это память компиляторов, не приложения. Первый прогон включал Desktop 6:02,
Core 3:16 и 18 модулей нового SDK. Из 36 поставляемых бинарников изменились 20;
14 исполняемых файлов расширений и два hook-helper сохранили прежние SHA256.
Добавленных/удалённых нет. В исходниках Modules receipt честно отмечает
`dirty:true`: во время замера там были только untracked локальные логи
`build/verification/`, без правок tracked production-кода. Исходный receipt
не исправлялся; каталог локально исключён из Git после проверки.

| Cargo graph Linux, включая dev/build | Workspace packages | Resolved packages | Edges |
|---|---:|---:|---:|
| Desktop | 6 | 681 | 2005 |
| Core | 5 | 218 | 510 |
| Modules | 38 | 325 | 885 |

Это размер resolve graph, а не число свежих компиляций. Полный receipt:
`build/performance/subagent-panel-20261103/report.json` и `run-*/build.log`.

Код готовой поставки:

- Desktop `520e94a115b40f07c29f7df120d82b7aa7ec1cd3`;
- Core `66bec470f6680aa7377ba9cff5f68cfcbf4a9126`;
- Modules `ef9c63816a400dadaab1ef6884cc198dbd31ed8a`;
- Rust contracts/SDK pin `9ce93629e0720a4107806f86a22b89e7095ae5a1`.

ELF Native SHA256:
`cd4eddbc7220ee5119b47cd997d0b0901b6653d7d57aa3d9e98d859d8ede076e`.
Документ добавлен после сборки и не меняет исполняемый код или готовую поставку.

## Пределы evidence

Readonly документ загружается лениво, использует существующий ориентир окна
200 строк / около 8 MiB, кроме неделимой страницы и видимых крупных строк,
и один физический history request. Автоматическое обновление связано
с lifecycle, без постоянного polling. При позднем binding, меняющем старую
строку, Core может выполнить полную проверенную перепроекцию истории.
Сравнимые RSS/PSS на 5k/50k и долгоживущие утечки в этом заходе не измерялись;
экономия RAM не заявляется. Native restart отдельно не проверялся; cold
storage projection и разбор сохранённого target покрыты Rust tests.

Свежая ошибка 111 в просмотренных сохранённых сессиях не подтверждена.
Найден запуск тестов без записанного результата/settlement, отмеченный
recovery как interrupted. Последний записанный факт не доказывает момент
падения или бесконечное выполнение. Исходная причина по этим данным неизвестна;
реальные команды повторно не запускались. Подробности и персональные пути
остаются в ignored локальном forensic receipt.
