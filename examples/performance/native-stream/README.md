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
держит активный ход без дельт ответа. В отдельном профиле — 20 синтетических
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

Постоянная проверка wire-сценария без GUI/сборки:

```sh
python3 examples/performance/native-stream/status_demo.py --check-fixture \
  --output build/verification/native-status-smoke-new
```
