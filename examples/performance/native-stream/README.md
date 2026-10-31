# Native: поток текста без завершающего snapshot

Постоянный синтетический стенд для coalescing соседних AssistantTextDelta и
root-thread routing. Он запускает **готовый** `proteus-native` через публичные
`--origin`/`--token`; Core, модули, платные модели и сборка не нужны. Transport
адаптирован из [delivery fixture](https://github.com/Qweasd123tg/proteus-modules-dot/tree/638c60b79f7ed4de76c84b3cb024b8bfdbff0df6/foundation/2026-10-27/adjacent-text-native-window).
Это проверка поведения, не замер производительности или доказательство того,
что все 64 события обязательно попадут в один UI batch.
MIT notice исходного transport сохранён в [LICENSE-MIT.fixture](LICENSE-MIT.fixture).

Создайте отдельную background-gui session и read-only watcher по skill
`background-gui`. Запустите runner **внутри** уже созданного display через
`bgui.py run`, не `exec`: runner держит процесс до команды stop. Пример из
корня Desktop (пути к skill и ready portable замените своими):

```sh
python3 /absolute/background-gui/scripts/bgui.py run SESSION --cwd /absolute/Proteus-desktop -- \
  python3 /absolute/Proteus-desktop/examples/performance/native-stream/run.py run \
  --native /absolute/ready/Proteus/proteus-native \
  --output /absolute/Proteus-desktop/build/verification/native-stream-current \
  --timeout 600
```

Output создаётся заново, существующий каталог не перезаписывается. Он содержит
отдельные HOME/config/cache/workspace/session и пустой каталог расширений.
Wayland, D-Bus и runtime display наследуются от background-gui. Чужие профили
и сессии не используются. Сервер слушает только `127.0.0.1` на случайном порту
со случайным Bearer token, валидирует пути/параметры и ограничивает очередь,
trace, размер body и число запросов. Всё работает на Python standard library.

Дождитесь `READY` в stdout/state.json и видимой строки `NATIVE STREAM READY`.
`READY` означает подключение SSE, а не presented frame. Затем с обычного
терминала продвигайте сценарий; следующую команду посылайте после `STAGE ...`
в stdout/state.json и проверки текущего экрана:

```sh
python3 examples/performance/native-stream/run.py advance --output OUTPUT --stage burst
python3 examples/performance/native-stream/run.py advance --output OUTPUT --stage barrier
python3 examples/performance/native-stream/run.py advance --output OUTPUT --stage continuation
python3 examples/performance/native-stream/run.py advance --output OUTPUT --stage hold
```

- `burst`: активный root turn берётся из `root_thread_id` начального snapshot;
  root TurnStarted повторно **не** отправляется, как при подписке после его
  записи в журнал. Затем 64 дельты одного message_id: индексы 0–31,
  коррекция с обрезанием до UTF-8 byte offset после восьмого фрагмента,
  затем индексы 33–63. Между половинами — дочерние TurnStarted и дельта
  **с тем же message_id**: `CHILD MUST NEVER APPEAR` не должна попасть в ленту
  или перехватить дальнейший root stream. Коррекция содержит кириллицу и 🦀.
  На этой стадии проверяйте screenshot: message Copy скрыт у streaming строки.
- `barrier`: отдельный ToolCallRequested `barrierprobe`, без исполнения.
  Весь первый текст должен оставаться видимым перед карточкой инструмента.
  Tool barrier снимает streaming и открывает message Copy для первого текста.
- `continuation`: дописывание того же message_id после инструмента, затем
  отдельное сообщение `DISTINCT MESSAGE: другой ответ 🌍`. Первое сообщение
  обновляется на прежнем месте, второе остаётся отдельной строкой.
  Появление второго сообщения снимает streaming с первого: его уже можно Copy.
- `hold`: только ToolCallRequested `freezeprobe`, без исполнения. Он снимает
  streaming со второго сообщения, открывая message Copy для обеих строк.
  Текст не заменяется: потерянные дельты этот barrier восстановить не может.

Сценарий **никогда** не отправляет AssistantMessageCompleted, второй snapshot
или terminal execution update. На каждой стадии экран держится до следующей
явной команды. Поэтому завершение не может исправить потерянные дельты перед
проверкой. Snapshot начальной строки остаётся единственным.

Сохраните screenshots через background-gui. Для точного текстового oracle
скопируйте нужное assistant сообщение действием Copy в UI и сохраните
**изолированный** clipboard display в файл без добавления перевода строки
(например, `wl-paste --no-newline` внутри того же display, если wl-clipboard
установлен). Не подставляйте expected-файл вместо реального clipboard.
Проверьте байты, пока стадия ещё активна:

```sh
python3 examples/performance/native-stream/run.py check-copy --output OUTPUT \
  --stage barrier --row primary --file OUTPUT/clipboard-barrier.txt
python3 examples/performance/native-stream/run.py check-copy --output OUTPUT \
  --stage continuation --row primary --file OUTPUT/clipboard-primary.txt
python3 examples/performance/native-stream/run.py check-copy --output OUTPUT \
  --stage hold --row secondary --file OUTPUT/clipboard-secondary.txt
```

На `hold` также можно повторно проверить `--row primary`: текст соответствует
первому сообщению после continuation. На `burst` используйте screenshot;
check-copy принимает только стадии, где message Copy доступен для строки.

Сравнение точное, без trim и нормализации. `copy-*.json` записывает результат,
время, SHA256 и число байтов; expected-*.txt содержит эталон. `trace.json`
хранит исходные payload и отметки записи SSE; `state.json` — SHA256 Native и
скриптов, PID, доставленную стадию и ошибки. `launcher`/`launcher_sha256`
описывают переданный `--native` файл. После подключения SSE runner отдельно
читает `/proc/PID/exe` и записывает `executable`/`executable_sha256` реального
запущенного ELF, поэтому одинаковые wrapper scripts не скрывают разные сборки.
Wrapper должен делать `exec` Native в своём PID: оставшийся интерпретатор
вызывает ошибку, а не выдаётся за приложение. Эти receipt не заменяют проверку
геометрии/порядка строк по настоящему окну.

```sh
python3 examples/performance/native-stream/run.py advance --output OUTPUT --stage stop
```

Stop, SIGTERM/SIGINT и общий timeout завершают только собственную Native
process group, закрывают сервер и сохраняют trace. После этого main останавливает
свою background-gui session. Нормальный stop сам по себе не означает PASS:
требуются сохранённые screenshots и успешные Copy receipts. Проверка самого
транспорта и контроллера без GUI/сборки:

```sh
python3 examples/performance/native-stream/test_fixture.py
```

## Ожидание ответа, прокрутка и кнопка вниз

`status_demo.py` использует тот же HTTP/SSE transport и готовый Native, но
держит активный ход без дельт ответа, пока не задан `control/answer`. В отдельном профиле — 20 синтетических
сообщений и последний вопрос. Расширения пустые; Core и модели не запускаются.
Запускайте внутри собственной background-gui session, как основной runner:

```sh
python3 /absolute/background-gui/scripts/bgui.py run SESSION --cwd /absolute/Proteus-desktop -- \
  python3 /absolute/Proteus-desktop/examples/performance/native-stream/status_demo.py \
  --native /absolute/ready/Proteus/proteus-native \
  --output /absolute/Proteus-desktop/build/verification/native-status-current \
  --timeout 600
```

После `READY` проверьте окно: «Думаю» находится после последнего вопроса и
прокручивается вместе с сообщениями. Прокрутите вверх: кнопка возврата вниз
показывает три движущиеся точки, при наведении — стрелку. Клик возвращает к
последнему вопросу и «Думаю». При выключенной/уменьшенной анимации точки статичны.

```sh
touch OUTPUT/control/finish
# Дождаться STAGE finished: «Думаю» исчезает, execution становится success.
touch OUTPUT/control/stop
```

`finish` оставляет окно открытым для сравнения, `stop` закрывает только этот
Native и transport. SIGINT/SIGTERM и timeout также выполняют очистку.
Допустимый timeout — 10–600 секунд. Output должен быть новым каталогом;
`state.json` фиксирует PID и SHA256 реального ELF, `trace.json` — доставленные
события. Bearer token в state/stdout не записывается. READY подтверждает
доставку событий, а вид и геометрию подтверждают только screenshots окна.

Чтобы воспроизвести сдвиг при завершении ответа, сначала прокрутите к середине
или наверх и оставьте видимый текст на месте. Затем отправьте фиксированный
многострочный ответ внизу:

```sh
touch OUTPUT/control/answer
# Дождаться STAGE writing; проверить, что видимый текст не сдвинулся.
touch OUTPUT/control/complete
# Дождаться STAGE completed: AssistantMessageCompleted уже доставлен,
# но execution всё ещё running. Проверить положение того же текста.
touch OUTPUT/control/finish
# Дождаться STAGE finished: execution success. Проверить положение снова.
touch OUTPUT/control/stop
```

`answer` отправляет один настоящий `AssistantTextDelta` с offset 0; `complete`
передаёт `AssistantMessageCompleted` с теми же message id и точным текстом,
без повторного snapshot. Каждый control применяется один раз. `complete`
необязателен: `finish` из writing сначала отправит completion, затем success;
без `answer` сохранится исходный thinking-only сценарий без assistant-сообщения.
Разрешение completion включено только для status demo; основной delta oracle
по умолчанию по-прежнему запрещает completion и replacement snapshot.

Для проверки изменения высоты viewport при завершении добавьте при запуске
`--completed-plan`. До `READY` сценарий отправит настоящий `ToolCallRequested`
для `update_plan` с двумя completed-шагами, затем `ToolFinished` с успешным
результатом. Пока execution running, над вводом видна полоса «План 2/2».
Раскройте план, прокрутите к середине или наверх и выполните ту же цепочку
`answer` → `complete` → `finish`: при `complete` план ещё виден, при `finish`
он исчезает. Положение читаемого текста должно сохраняться и при исчезновении
свёрнутой полосы, и при исчезновении раскрытого плана. Без флага план не
создаётся; это всё ещё синтетический transport без запуска tools/Core.

Постоянная проверка wire-сценария без GUI/сборки:

```sh
python3 examples/performance/native-stream/status_demo.py --check-fixture \
  --output build/verification/native-status-smoke-new
```

## Ширина блоков, горизонтальная прокрутка и Copy

`code_blocks.py` показывает одно синтетическое assistant сообщение с пятью
завершёнными fences: Rust из двух строк с финальным LF, короткие `text` и `md`,
три строки с ASCII/CJK/табами/эмодзи и Rust строку длиннее 180 колонок.
У CJK-строки больше UTF-8 байтов, чем у ASCII, но она визуально короче:
ширину нельзя считать через длину строки. Короткие блоки должны занимать ширину
содержимого, длинный — ограничиваться чатом. Индикаторы скрытого текста нужны
только на соответствующем краю при горизонтальной прокрутке; номера строк
остаются на месте, вертикальное колесо продолжает прокручивать чат.

Запускайте внутри отдельного background-gui display с watcher, как основной
runner. Для Markdown нужен **готовый каталог установленных расширений**:
Из указанного `--extensions` стенд берёт только готовый `markdown-render` и
создаёт symlink в частном `OUTPUT/renderer-extensions`; именно этот каталог
передаётся как `PROTEUS_EXTENSIONS_DIR`. Остальные расширения и их badges
не загружаются. Это проверка кодовых блоков с Markdown, а не расхода всей
сборки со всеми расширениями. HOME/config/cache/session/workspace находятся
только в новом output; пакеты не собираются и не устанавливаются.

```sh
python3 /absolute/background-gui/scripts/bgui.py run SESSION --cwd /absolute/Proteus-desktop -- \
  python3 /absolute/Proteus-desktop/examples/performance/native-stream/code_blocks.py \
  --native /absolute/Proteus-desktop/build/Proteus/proteus-native \
  --extensions /absolute/Proteus-desktop/build/Proteus/share/extensions \
  --output /absolute/Proteus-desktop/build/verification/native-code-blocks-current \
  --timeout 600
```

`READY` подтверждает доставку первого snapshot, а не геометрию кадра.
Изначально стадия `initial` держится без автоматического обновления. При
1440×1000 сообщение рассчитано примерно на 700 px высоты; если из-за настроек
шрифта нижний длинный fence не виден, прокрутите чат вниз. Проверьте каждый
блок и скопируйте его **кнопкой блока**, включая финальный LF и табы.
Точный исходник каждого fence сохранён в `expected/initial/{rust,text,markdown,unicode,long}.txt`;
`message.txt` содержит всё сообщение и предназначен для отдельного message Copy.
Не используйте expected-файл вместо реального clipboard. Например, после
копирования короткого Rust блока сохраните clipboard из того же display:

```sh
python3 /absolute/background-gui/scripts/bgui.py exec SESSION -- wl-paste --no-newline
# Сохранить stdout команды wl-paste из ответа bgui в OUTPUT/clipboard-rust.txt,
# без JSON-обёртки bgui и без добавления LF; затем сравнить:
cmp OUTPUT/expected/initial/rust.txt OUTPUT/clipboard-rust.txt
```

Только после начальных Copy/геометрии запустите единственное явное обновление:

```sh
touch OUTPUT/control/update
# Дождаться STAGE updated и нового кадра; Rust теперь из трёх строк.
```

Обновляется тот же message_id через существующие SSE delta и tool barrier,
затем execution переходит в success. Второй snapshot/AssistantMessageCompleted
не отправляется. Все fences закрыты до и после обновления, автоматических
замен больше нет. Повторно проверьте Copy: новый короткий Rust должен совпасть
с `expected/updated/rust.txt`, остальные — со своими updated-файлами.
Это проверяет обновление сохранённого исходника без копирования текста на
каждом render; старое значение нельзя подставлять из начального oracle.
После обновления в ленте также появляется синтетический tool barrier.

```sh
touch OUTPUT/control/stop
```

Stop, SIGINT/SIGTERM и timeout (10–600 секунд, по умолчанию 600) завершают
только собственный Native и transport, сохраняя `trace.json`, `state.json`,
`native.log`, SHA256 launcher и фактического ELF. State также хранит исходный
installed-каталог, частный каталог, путь и SHA256 manifest/ELF Markdown renderer.
Секретный token в state/stdout
не выводится. Затем остановите собственную background-gui session. Сервер —
тот же локальный mock, Core, модели и реальные журналы не используются.
Wire/source smoke без GUI и сборки:

```sh
python3 examples/performance/native-stream/code_blocks.py --check-fixture \
  --output build/verification/native-code-blocks-smoke-new
```

Для проверки углов раскрываемых tool-блоков добавьте к запуску `code_blocks.py`
флаг `--tool-previews`. После пяти fences snapshot содержит три смежные
завершённые system tool-строки: `read_file` (`preview-short`, короткий result),
`exec_command` (`preview-long`, subject из 24 строк и result из 80 строк),
`grep` (`preview-search`, короткий result). Это готовые синтетические DTO;
команды не исполняются. Точные subject/result и snapshot находятся в
`expected/tools/`; файлы Copy для пяти fences сохраняются без изменений.
Сразу после трёх tools следует отдельное assistant-сообщение
**«AFTER TOOL GROUP · неподвижный маркер»** (message id заканчивается `03e7`,
отличается от обновляемого code-сообщения). Его Y-позиция позволяет заметить
скачок в последний кадр раскрытия/сворачивания цепочки; точные UTF-8 байты без
финального LF сохранены в `expected/tools/after-group-marker.txt`.

Для воспроизводимой геометрии доступны `--scale` (75, 80, 90, 100, 110, 125,
150, 175, 200; по умолчанию 100) и `--surface-radius` (0–32 px при 100%,
по умолчанию 16). Например, `--tool-previews --scale 125 --surface-radius 32`
проверяет увеличенные промежутки и хорошо заметные углы. До запуска Native
стенд всегда записывает только `view.scale` и `view.surface_radius` в частный
`OUTPUT/xdg-config/dev.proteus.native/preferences.json`; остальные значения
наследуют serde defaults. Host preferences не читаются и не меняются.
`state.json` сохраняет заданные значения, путь и SHA256 этого файла; тот же
receipt создаётся при `--check-fixture`, который не подтверждает GUI-геометрию.

Для сохранения кадров до и после раскрытия используйте постоянный скрипт
`capture_motion.py` в уже запущенной private background-gui session. Выберите
координаты по свежему кадру; output должен быть новым каталогом:

```sh
python3 examples/performance/native-stream/capture_motion.py \
  --helper /home/qweasd123tg/.codex/skills/background-gui/scripts/bgui.py \
  --session SESSION --output build/verification/finish-anchor-20261031/motion-new \
  --click X Y --frames 12 --interval .08
```

`frame-000.png` — baseline до клика. Затем один helper click выполняется
параллельно циклу screenshot. Промежуточные кадры короткого перехода могут
не попасть в захват из-за сериализации helper; сравнивайте также endpoints.
PNGs сохраняются как получены, без обработки; stdout/stderr каждой команды —
в отдельном `.log`. `capture.json` содержит monotonic_ns начала/конца команд,
exit status клика и screenshots, ошибки и пути PNG. Это границы исполнения
команд, **не точное время display capture**: helper сериализует операции
session, а его overhead может увеличить реальный интервал. Статус
`commands-completed` подтверждает только успешные команды, не правильность UI.
Скрипт не меняет production-анимации, не использует host input и не закрывает
session; при ошибке сохраняет уже полученные кадры и metadata.

После `READY` прокрутите до конца. Если включены «Компактные цепочки
инструментов» в настройках чата, нажмите строку **«Инструменты · 3»** со
стрелкой справа: откроются три названия. Для обычного раскрытия нажмите строку
`read_file`, затем её стрелку повторно для закрытия; короткий subject/result
должен сохранять округление по ходу движения. Нажмите `exec_command`: сначала
видны урезанные subject и result. У каждого длинного блока есть собственная
кнопка **«Показать больше · ещё строк: …»** над ним — по очереди раскройте subject, прокрутите
к result, раскройте его и проверьте нижние углы. **«Свернуть»** возвращает preview.
Закройте `exec_command`, затем сверните/раскройте «Инструменты · 3», проверяя
также углы короткого блока при движении всей цепочки. При выключенных compact
chains три строки видны сразу; включите настройку для группового варианта.

Только после начальной проверки выполните `control/update`: обновится исходное
assistant-сообщение с тем же id, а три tool-строки сохранят порядок и содержимое.
После assistant-маркера добавится прежний `codeblockfreeze` barrier;
исходная компактная цепочка останется **«Инструменты · 3»**. Стенд не присылает replacement snapshot. Wire/source
проверка варианта:

```sh
python3 examples/performance/native-stream/code_blocks.py --check-fixture --tool-previews \
  --output build/verification/finish-anchor-20261031/tool-previews-wire
```

Для проверки I-beam над форматированным текстом `status_demo.py` также принимает
`--extensions ABS_SELECTED_DIR`. Передавайте только каталог уже готовых нужных
рендереров (например, приватный `renderer-extensions` стенда code_blocks), чтобы
постоянные badges не требовали настоящего backend. По умолчанию каталог пустой.
Это не включает сборку расширений. При активном «Думаю» проверьте два цикла:
колесо вверх → точки → наведение (стрелка) → вниз → снова вверх → точки,
затем `control/finish` → стрелка. Наведение в пустом поле справа от текста
не должно включать I-beam; drag можно закончить за строкой.
