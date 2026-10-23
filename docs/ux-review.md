# UX-аудит Proteus по исходникам Zed

23 октября 2026. Сопоставлены выбранные сценарии вкладок, чата, Git/diff,
настроек, меню и фокуса. Образец — локальные исходники Zed
`03efd49220ba`. Это целевая проверка конкретных механизмов, а не полный
аудит Zed или заявление о полном совпадении интерфейсов.

Правила продукта находятся в [DESIGN.md](../DESIGN.md). Этот документ
фиксирует причины изменений, общие примитивы и пределы проверки текущей пачки.

## Общие примитивы и миграции

| Примитив | Где используется | Общее поведение |
| --- | --- | --- |
| `ui::picker`, публичный `Picker` | Model/Branch и процессные страницы выбора | Поиск, виртуализированные строки, текущий value, курсор, клавиатура, прокрутка и фокус. Высота следует числу результатов, без пустых восьми строк. |
| `ui::reorder`, публичный `Sortable` | Settings Extensions, Hooks, произвольные процессные карточки | Удержание и перенос всей строки, preview полной ширины, анимация соседей, стабильные ids; один `{id,before}` после drop. Ctrl+Shift+↑/↓ доступны на самой строке. |
| `ui::selection` | Меню exclusive choice, Picker, списки, дерево, навигация | Выбранное значение сохраняет плашку `sidebar_accent`, пока курсор/hover движется отдельно. `MenuItem.selected` отделён от boolean `checked`. |
| `motion::popup` / `preview_reveal` | Контекстные меню, Model/Project picker, инструменты чата | Общий mask reveal и шеврон. Размер popup известен сразу, положение не переворачивается посреди анимации. Show more сохраняет закрытое превью и scroll anchor. |
| `TooltipAnchor`, `fade::Caption` | Вкладки, чипы, списки, дерево, сессии | Подсказка привязана ко всему контролу, ограничена по ширине и переносит текст. Полностью видимая подпись не повторяется. |
| `tabs`, `tabs::history` | Чаты и любые extension documents | Preview, pin, MRU, закрытие/возврат, drag и автоскролл не зависят от вида документа. |
| `ui::diff` / общий parser | Git, commit, apply_patch | Один split/unified renderer, исходный patch для копирования, номера строк и metadata. |
| `ui::state`, публичный `Loading` | Процессные поверхности и клиент | Загрузка имеет отложенный spinner; пустой результат остаётся отдельным состоянием. |

`ui::divider` сводит sidebar, split panes и graph lanes к линии и hover/drag
маркеру 1px с невидимой зоной захвата 7px по всей высоте. Cross-highlight
возникал из-за старого `SidebarDrag` key: `ColumnDrag` не объявлял свой source.
Теперь он вызывает `drag::start` со стабильным entity/column key; новый жест
также очищает прежний key. Общий gate и native проверка прошли: последовательный sidebar → column drag не меняет pixels sidebar; hover красит ровно 1 px, resize работает ниже строк таблицы.

## Найденные причины и изменения

**Задержка открытия Git.** Общий SDK на каждый `render` заново строил все
сохранённые поверхности в порядке `BTreeMap`. Тяжёлые `document:*` попадали
перед запрошенной `panel`. Теперь обычный запрос строит одну поверхность;
после `action` сначала отвечает инициатор, затем обновляются остальные.
Session switch очищает прежние root/state/pending до `session_changed`,
а close снимает старый action sequence. Исправление действует на все пакеты.

**Устаревший working diff.** Статус и количество `+N/−M` не определяют
содержимое файла: одинаковая по длине правка могла оставить прежний patch в
кэше. Core возвращает opaque `revision` пути на основании HEAD, index и
метаданных файла. Status и diff используют один контракт; изменившийся либо
неизвестный token не кэшируется. Пакет сохраняет только совпадающие revision,
поэтому правка одного файла не выбрасывает остальные patches. Это stat token,
не криптографический hash; точность зависит от файловой системы.

**Ошибки diff и графа.** `No newline` разрывал одну replacement-пару на две
односторонние строки. Метаданные теперь прикреплены к соответствующей строке.
Plain unified patch из нескольких файлов разделяется по `---/+++` после
исчерпания объявленных hunk counts; похожий исходный код не считается header.
Новый graph query и Aa больше не выполняют неявный Next. Inspector и short-hash
вкладка разделяют один неизменяемый ответ commit вместо повторного HTTP.

**Черновики и attachments.** Ошибка Send могла потерять первое сообщение
или соединить его изображения со вторым текстом. Полный draft возвращается
атомарно в пустой composer либо сохраняется отдельно; Restore также сохраняет
занятый composer. Send отражает реальную готовность, свободный ответ обновляет
Next/Send, Escape вложенного меню не отклоняет approval. Выбор/drop/paste
изображений проверяет общий бюджет до чтения и подготавливается в фоне;
поколение draft не позволяет завершению загрузки вставить данные в другой
черновик. Полный исходный tool output копируется отдельно от ограниченного
по объёму представления.

**Вкладки и фокус.** Закрытие фоновой вкладки больше не переводит фокус в
другую панель. Вместо одного `previous` хранится полная MRU. Автоскролл учитывает
перестановку того же active id и промежуточные child slots закрывающихся tabs.
Ctrl+W/Ctrl+F4 защищают pins; Ctrl+Shift+T возвращает regular document/chat,
если он доступен. Чат сначала резюмируется, затем восстанавливается его вкладка.
История закрытий остаётся локальной для текущего окна.

**Настройки и маленькие controls.** У коротких enum убрана принудительная
ширина 200px, у каждого multi-choice checkbox — фиксированная колонка 220px.
Общий Picker сохраняет query/cursor по identity, обновляет callback при смене
action, получает явный focus по id и владеет своей навигацией раньше общих
view keys. Initial focus следует первому видимому control в порядке дерева,
пропуская скрытые вкладки и закрытые disclosure.

**Reorder: дефекты, обнаруженные native проверкой.** Первый сценарий
подтвердил перенос, ghost, промежуточные позиции и сохранение порядка во время
600ms process ack. Отдельно выявлены неработавший Escape и drag вместо
удерживаемой дочерней Button. Подготовлены общий root cancellation/hold epoch
и геометрические exclusions ближайшего списка: native listeners GPUI
регистрируются в paint, поэтому прежний prepaint bubble marker срабатывал
слишком поздно. Повторная native проверка прошла: Esc сохраняет порядок, hold вложенной кнопки не переставляет строку, её click работает, Ctrl+Shift+↓ и drop доходят до отдельного процесса;
их нельзя считать подтверждёнными по первому успешному drop.

## Граница расширений

Пакет — самостоятельный stdio process с `extension.json` и публичными
JSON-lines сообщениями. Клиент исполняет общие focus, keyboard, sizing,
selection, animation и drag правила; пакет передаёт данные и обрабатывает
свои действия. Нативные ветки по id конкретного расширения не добавляются.

Новые контракты описываются вместе со schema, SDK, `PROTOCOL.md` и producers:
`widgets.picker.v1`, `widgets.sortable.v1`, `widgets.selection.v1`,
`widgets.status.v1`. После установки поддерживающего их клиента отдельный
пакет может менять UI из этих примитивов без пересборки host. Добавление
совершенно нового нативного примитива требует согласованного обновления
клиента. Требуемые capabilities объявляются явно.

## Опорные места Zed

Все ссылки относятся к `03efd49220ba`; GPL-решения адаптируются внутри
GPL-клиента, общий Apache SDK сохраняет собственную реализацию протокола.

| Решение | Источник Zed |
| --- | --- |
| Tab MRU, active scroll, close/pins | `crates/workspace/src/pane.rs:1485–1664`, `:2150–2200` |
| Закрытые regular items, shortcut defaults | `pane.rs:4921–4950`, `workspace.rs:3321`, `assets/keymaps/default-linux.json:502–503,695` |
| Graph query отдельно от next/previous | `crates/git_ui/src/git_graph.rs:2242–2286`, `:2469–2510` |
| Разделение source buffers и line-end metadata | `crates/git_ui/src/commit_view.rs:390–425`, `:1114–1165` |
| Failed-send draft restore, Send readiness | `crates/agent_ui/src/conversation_view/message_queue.rs:121`, `thread_view.rs:3005–3031,6456` |
| Tool input/output и полный источник Copy | `thread_view.rs:490–531,9490`, `crates/ui/src/components/disclosure.rs:95` |
| Picker query/cursor/scroll | `crates/picker/src/picker.rs` |
| Intrinsic enum width, bounded tooltip | `crates/settings_ui/src/components/dropdown.rs:92–141`, `crates/ui/src/components/tooltip.rs:194` |

## Проверки и измерение задержки

Исторический общий gate до повторной проверки native fixes:
4 client-common, 142 client, 79 extension Rust, 14 Python tests;
103 реальных protocol messages; 18 Core git HTTP tests. Это числа этого
этапа, не обещание итогового packaged gate. Форматирование и лицензии проверяются
общим gate; после последних правок выполняются повторные затронутые проверки.

Для двух снимков настоящего Git process измерены пять чередующихся запусков
на частном loopback HTTP mock: 40 synthetic files, искусственные 25ms на
каждый `/workspace/diff`, контролируемое истечение TTL без background polling.
Медиана ответа panel — **1048.401ms → 6.502ms**. До ответа panel старый
снимок выполнял 40 скрытых retained-document diffs, новый — 0.
Это измерение протокольного ответа на искусственной задержке, не latency
пользовательского Git или native frame. Снимки содержат и другие изменения;
причинное свидетельство здесь — порядок и количество HTTP запросов.
Артефакты текущего запуска: `/tmp/proteus-zed-sdk-http-perf/results.json`
и `README.md` с hashes, условиями и всеми образцами.

Отдельное изолированное native окно подтвердило failed-send draft swap,
соответствие текста и bytes изображений, лимит четырёх clipboard images,
ограниченный tooltip с одинаковыми pixels при двух точках hover и отсутствие
повторяющих подпись hints у коротких pinned/regular tabs. Использовались
synthetic sessions, fake model и собственный Xvfb; реальные сессии не читались.
Это проверка development snapshots. Последний portable, проверки всех его
бинарников и перезапуск пользовательского приложения завершаются отдельно.

## Что остаётся и что пока не доказано

- Граф ищет среди загруженных записей. Полный repository-wide search требует
  app-server контракта и независимости поиска от pagination.
- Внешнее изменение списка branches без смены HEAD может оставить cache
  устаревшим; нужен общий repo-change epoch/watcher. Native repro не выполнялся.
- Eager syntax/width measurement больших diffs может задержать UI thread;
  это source-level гипотеза, требующая профиля на большом synthetic patch.
- Tool code/output пока не полностью использует общий syntax-aware горизонтальный
  code viewport. Полезен следующий общий примитив, вместо отдельных решений пакетов.
- В Zed pins вынесены из scrolling strip, preview хранится per pane. В Proteus
  пока общая полоса и один preview на окно; это не менялось молча.
- Composer/SavedDraft и closed-tab history не обещают сохранение после явного
  закрытия соответствующей entity или перезапуска процесса.

## Дополнительные native проверки текущей итерации

- `/tmp/proteus-zed-audit-native-20261023/divider-validation.json`: тонкая линия и изоляция источников drag.
- `sortable-validation.json`: Escape, вложенная кнопка, keyboard reorder и задержанный authoritative view.
- `settings-order-validation.json`: настоящие Extensions и Hooks меняют порядок удержанием всей строки; Ctrl+W сохраняет несохранённый draft настроек.
- `/tmp/proteus-zed-draft-native-20261023/selection/selection-native-validation.json`: selected сохраняется при hover/keyboard другого пункта, Settings/Picker используют один цвет, bool menu сохраняет Check.

После этих проверок ещё добавлены новый общий Search, retained tail при
сворачивании чата, постоянная задержка tooltip, composer activity dock и
процессные rich content surfaces. Их заключительная проверка описана ниже;
прежние результаты сами по себе не доказывают готовность этих механизмов.

## Composer activity dock: новая итерация

По уточнению владельца project/context, публичные `composer_start`/`composer_end`,
pending questions/approvals и реальный текущий plan собраны в общей компактной
полосе, пристыкованной к верху composer. Общий `chat/activity_dock.rs` хранит
выбор по stable request id, показывает triggers всех активных сущностей и
раскрывает подробности общими `motion::chevron/reveal/toggled`. Начатый ответ
не вытесняется новым pending; интерактивные controls не дублируются в thread.
Plan проецируется из текущего transcript `AppTranscriptTool update_plan`,
без придуманных шагов. Public places сохраняют имена, retained badge entities
и capability: пакет расширения не переносит свою UI-логику в клиент.

Enter/Esc/цифры теперь адресуют видимую выбранную секцию; Enter сначала
раскрывает свёрнутый pending. Меню и free-answer Input сохраняют собственную
клавиатуру, исходные send/drop/attachments пути остаются общими. Подробности
ограничены 40% viewport и 360 scaled px, strip — 96 scaled px с прокруткой.
Источник Zed: `conversation_view/thread_view.rs:3878`, `:4736`, `:4839`.
По просьбе владельца добавлены единая strip селекторов и общая плавная анимация.
Три новые логические регрессии, targeted rustfmt и diff check подготовлены;
Cargo/native нового dock на момент добавления раздела ещё ожидали общего gate;
результат повторной проверки приведён ниже.

Self-review дополнительно исправил anchor: в закреплённом снизу composer
details располагаются над strip/editor и растут вверх. Сама strip и её
compact plan summary не меняют геометрию от open/close. Изолированная fixture
`/tmp/proteus-zed-dock-native-20261023` подготовлена с public DTO и process
selectors; финальная проверка выполнена на другом компьютере, как описано ниже.

## Заключительная native проверка 23 октября

На Fedora собран актуальный `fast` binary. Проверка выполнена в настоящем
GPUI-окне внутри Xephyr с программным Mesa renderer, через synthetic HTTP/SSE
и отдельные XDG-каталоги. Реальные сессии и модель не использовались. Это
проверка поведения native клиента, а не измерение производительности GPU
или проверка live provider. Локальные артефакты —
`build/verification/ui-2026-10-23/`.

Проверены Search в Settings и Commit Graph (Ctrl+F, очистка первым Esc,
закрытие вторым, узкая область); hover слабее selected в тёмной и светлой
темах; полная задержка новой tooltip и отсутствие повторения видимой подписи.
В dock сохранены независимые ответы двух запросов, цифровые shortcuts и
отправка; свёрнутый approval сначала раскрывается по Enter, Esc его меню
не отклоняет запрос. В окне шириной 800 px работают strip, clipboard image
и Send. После collapse отдельного инструмента, цепочки и subagent остаётся
пустой хвост; переход к концу его убирает.

Rich content проверен на JSON, цветном SVG, локальной ошибке SVG между двумя
здоровыми блоками, неизвестном языке и незавершённом fence. Copy возвращает
полный source; локальные Tabs и source toggle переживают виртуализацию
60 строк и смену чата. ZIP установлен и обновлён через native drop, renderer
выключен и включён без пересборки host. Optional skill не устанавливался.

В этой проверке найдены и исправлены четыре причины:

- Любое runtime notification ошибочно считалось пользовательским resize:
  первый preview останавливал следование концу. Теперь фоновые обновления
  только перемеряют строку, а сохранение места вызывают жесты и их action reply.
- Невалидный SVG превращался в ошибку всего процесса. Валидный envelope
  сохраняет view/session/sequence и направляет ошибку только своему блоку.
- Поздний hover-leave соседней кнопки отменял новую tooltip. Уход снимает
  только запрос своего якоря; переключение вновь выдерживает полные 500 мс.
- Нижняя кнопка сворачивания длинного tool output исчезала выше viewport.
  Общие «Показать больше»/«Свернуть» и Copy перенесены над результатом и
  аргументами; native сравнение подтвердило неизменность позиции при collapse.
  Это намеренное уточнение прежнего нижнего control по правилу стабильного
  заголовка из DESIGN.md, а не копирование его позиции из Zed.

Итоговый client gate: 165 tests + 4 client-common, включая регрессию адресации
невалидного content tree; Extensions: 84 Rust + 14 Python, 106 реальных wire
messages / 11 пакетов. Форматирование и лицензии — PASS. Portable требует
чистых commits и проверяется отдельно по `share/release-sources.json` и
упакованным бинарникам; нативная приёмка не заменяет этот шаг.

## Падение при смене папки проекта, 24 октября

Владелец обнаружил panic при нажатии «Открыть папку…» в плашке над composer.
Изолированное воспроизведение подтвердило `cannot update ProjectPicker while
it is already being updated`: `ProjectPicker::close` синхронно закрывал
Popover, его `on_open_change` повторно обновлял ещё занятый picker. До
системного диалога выполнение не доходило. Тот же путь использовали недавняя
папка, Enter в поиске и Esc.

Закрытие Popover перенесено в `window.defer`, после освобождения текущего
обработчика picker. Проверка выполнена в настоящем Wayland-окне на Niri с
отдельными XDG-каталогами, synthetic HTTP/SSE и приватным FileChooser portal:
успешный выбор тестовой папки передаёт её в `/new-session`, отмена сохраняет
чат, Esc, недавняя папка и Enter не завершают процесс. Все пять native
сценариев и client gate (165 + 4 tests) прошли. Приватный portal проверяет
асинхронные success/cancel ответы, а не внешний вид системного chooser.
Локальные артефакты: `build/verification/project-switch-2026-10-24/`.
