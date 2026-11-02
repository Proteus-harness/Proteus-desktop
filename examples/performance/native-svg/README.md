# Native SVG: смена источников и освобождение atlas

Постоянный стенд для **готовых** сборок Desktop. Cargo, Core, модели и реальные
сессии не запускаются. HTTP/SSE импортируется из
`../native-stream/fixture_server.py` с его
[MIT-лицензией](../native-stream/LICENSE-MIT.fixture); запуск, fingerprint ELF и
остановка Native используют helpers `../native-stream/run.py`. `/proc` sampler
импортируется из соседнего Core без его сборки.

В изолированный каталог extensions устанавливается ровно один маленький
Python-процесс `memory-svg`. Он реализует настоящий protocol-v4 content
contributor для format `text/markdown` и языка `memory-svg` и возвращает два настоящих `Node::Svg` 1280×280. Это не готовая
картинка, подмена native renderer или тест вызова Rust helper. Нативный host
разбирает/rasterize SVG и рисует через свой обычный GPUI atlas. Python не
использует сторонние библиотеки и не занимается rasterization. Вместо общего
Markdown renderer он принимает только точные синтетические fences корпуса;
другие Markdown сообщения отвергаются. Это нужно, поскольку без format provider
Chat показывает Plain и не делегирует вложенные языки.

16 детерминированных стадий меняют цвета и подписи SVG, сохраняя один
`message_id`, один fence языка `memory-svg` и stream/session/thread identity.
Каждая стадия заменяет историю правильным snapshot с возрастающим seq.
Последующая очистка удаляет единственное сообщение. Это snapshot-only тест
владения ресурсами, а не проверка delta/replay или модельного streaming.
Слишком ранние, повторные и пропущенные стадии отвергаются.

Запускать через `background-gui` в отдельном дисплее согласно AGENTS.md:

```sh
python3 examples/performance/native-svg/run.py \
  --native "$PWD/build/Proteus/proteus-native" \
  --output "$PWD/build/verification/native-svg-before"
```

`--native` может указывать portable wrapper, который делает `exec` Native в
том же PID. После SSE подключения фиксируется SHA-256 **реального ELF**, а не
только wrapper. Output должен быть новым внутри `build/verification`.
HOME, XDG, workspace, session, profile, extensions изолированы. Токен случайный,
скрыт в argv samples и launch receipt; renderer не пишет hello credentials.
Timeout 10–7200 секунд, по умолчанию 1200.

После `READY`:

1. Визуально убедиться, что видны обе разноцветные панели `SVG STAGE 00`.
   Проверить `state.json`: `rendered_stages` содержит `0`. Сам callback receipt
   не доказывает paint — это отдельная проверка запускающего агента.
2. После одинаковой idle-паузы отметить `mark-stage-00`.
3. Создать только `next-01`, дождаться **новой подписи на обеих панелях**,
   callback receipt и idle. Продолжить последовательно до `next-15`.
   Не отправлять следующую стадию до визуальной проверки предыдущей.
4. Отметить замер последней стадии, создать `clear`, дождаться пустого чата,
   отметить освобождение после одинаковой паузы.
5. Создать `stop`. Повторить на второй готовой сборке с теми же источниками,
   дисплеем, геометрией, renderer, scale, действиями и idle-паузами.

Пример ручного контроля (последовательно, с проверкой GUI между командами):

```sh
touch build/verification/native-svg-before/control/mark-stage-00
touch build/verification/native-svg-before/control/next-01
# … следующая команда только после проверки стадии 01 …
touch build/verification/native-svg-before/control/next-02
# … стадии 03–15 …
touch build/verification/native-svg-before/control/mark-stage-15
touch build/verification/native-svg-before/control/clear
touch build/verification/native-svg-before/control/mark-cleared
touch build/verification/native-svg-before/control/stop
```

`next-NN` — отдельный файл для каждой стадии; одновременно больше одного
нового файла запрещено. Предыдущая стадия должна иметь callback receipt до
продвижения. `clear` разрешён один раз после callback стадии 15. Не стирайте
и не переиспользуйте control-файлы. `mark-NAME` принимает каждый NAME один
раз; `phase` с коротким буквенно-цифровым именем меняет phase sampler.

Артефакты:

- `corpus.json`: одинаковый между before/after корпус, размеры, source и hashes
  всех 32 SVG. Его SHA-256 должен совпадать при сравнении.
- `rendered.jsonl`: callback stage, view, source_revision и точные SVG hashes;
  подтверждает доставку настоящих Node::Svg, но не paint. Максимум 512 строк.
- `state.json`, `trace.json`, `report.json`: прогресс, seq, ошибки, source /
  transport / sampler / wrapper / ELF hashes и именованные samples.
- `samples.jsonl`: PSS/RSS/CPU дерева Native с интервалом около 100 мс,
  включая synthetic extension child; Python driver, compositor и watcher
  исключены. Разделяйте PSS Native и process-tree; VRAM в PSS не входит.
- `native.log`: диагностика приложения. `native_verified` остаётся false;
  визуальные доказательства фиксируются отдельно запускающим агентом.

SIGINT/SIGTERM/stop завершают собственный Native. Linux subreaper принимает
осиротевший synthetic renderer, посылает ему TERM/KILL и вызывает waitpid;
renderer имеет собственную process group. Неожиданный выход Native является
ошибкой даже с кодом 0, все завершённые и аварийные запуски сохраняют отчёт.
Стенд не управляет GUI и не открывает watcher самостоятельно.

Тесты без GUI/Cargo:

```sh
python3 -m unittest discover -s examples/performance/native-svg -p 'test_*.py' -v
```

## Проверка реального paint

После запуска стенда на отдельном background-gui дисплее можно использовать
`tools/capture.py`, передав ID **его** сессии и ту же output-папку:

```sh
python3 examples/performance/native-svg/tools/capture.py \
  --session BACKGROUND_GUI_SESSION \
  --output "$PWD/build/verification/native-svg-before" \
  --settle 2 --stop
```

Helper последовательно проверяет 16 стадий с двумя SVG в каждой (32 источника),
затем очистку. Помимо callback он снимает настоящий PNG дисплея через
background-gui и требует не менее 50 000 пикселей каждого из двух ожидаемых
цветов панелей. Следующая стадия начинается только после успешной проверки
paint, одинаковой паузы (по умолчанию 2 секунды) и повторного скриншота.
После очистки оба цвета последней стадии должны отсутствовать. Геометрия
окна должна позволять обеим панелям быть достаточно видимыми; ошибку проверки
нельзя обходить слепым продвижением control-файлов.

`pixel-verification.json` сохраняет цвета, число пикселей, имена и SHA-256
скриншотов, начальную стадию, idle-паузу и SHA-256 самого helper. Для PNG
выбираются новые имена с порядковым суффиксом: существующие снимки не
перезаписываются. Запуск с уже созданным `pixel-verification.json` запрещён.
Если начало пройдено вручную, helper проверяет сохранённые `stage-NN.png`
предыдущих стадий; такие отличия последовательности надо указать в сравнении.
`--stop` останавливает стенд после последнего замера, без него он остаётся жив.

Только этому необязательному helper нужен Pillow для чтения пикселей;
это зависимость проверки, не Desktop или synthetic renderer. Он не меняет
снимки. Совпадение цветов подтверждает paint панелей, но не всю геометрию SVG,
освобождение GPU-ресурсов или отсутствие утечек. `native_verified` runner
по-прежнему не заменяет отдельную визуальную оценку. Корпус и исходники стенда
фиксируются до before; между before/after не меняйте корневые `.py` файлы,
helper и его параметры. Его подкаталог `tools` не входит в fingerprint
корневых `.py` стенда, поэтому hash helper фиксируется отдельно.
Перед следующей стадией helper ждёт подтверждения `mark-NAME` в
`marked_samples`: наличие control-файла само по себе ещё не подтверждает
замер. Это сохраняет последний SVG sample до очистки и остановки renderer.
