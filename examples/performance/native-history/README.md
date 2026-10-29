# Native: длинная история, запуск и память

Постоянный Linux-стенд запускает настоящий release Desktop и его дочерний Core
на синтетической canonical history. Для каждого размера и повтора создаётся
отдельный config/cache: сначала открытие без прикладного read-cache, затем новый
процесс с сохранёнными cache/preferences. Кэш ОС не сбрасывается.

Для UI-изменений используйте готовые Core, модули, расширения и уже
собранный `session_index_fixture`. Стенд получает их явными путями и сам не
собирает Core. Найдите готовый seeder в сохранённых артефактах или
`Proteus-core/target/release/examples/session_index_fixture`; пересборка
Desktop не требует его обновления. Для ручной проверки можно использовать
ранее созданные синтетические чаты. Этот runner создаёт собственные корпуса
через готовый seeder, поэтому не направляйте его на реальные сессии.

Обновление только клиента, из Proteus-desktop:

```sh
python3 scripts/portable.py --no-archive
# либо взять готовый runtime из другой поставки:
python3 scripts/portable.py --base /absolute/ready/Proteus --no-archive
```

Если нужного seeder нет, не запускайте сборку Core автоматически. Целевая
сборка генератора оправдана, когда изменён его код/формат истории и требуется
новый корпус, либо владелец явно поручил получить новую fixture таким способом.
Тогда команда из Proteus-core:

```sh
CARGO_BUILD_JOBS=2 cargo build --locked --release -p proteus-core --example session_index_fixture
```

Сохраните до обновления всю готовую portable-папку вместе с executable,
runtime-assets и provenance. Для сравнения нужны одинаковые обычные release
profiles, Core, seeder, modules, extensions и параметры корпуса; меняйте только
исследуемые компоненты. Профиля `--fast` больше нет. Корпус содержит
5 000/50 000 отдельных сообщений, а не многократную замену одного сообщения.
`--text-repeats 4` задаёт примерно 1,96/19,69 МБ UTF-8 текста плюс canonical metadata.
Выбор компонентов и отдельный замер стоимости сборки описаны в
[BUILDING.md](../../../BUILDING.md) и
[build-cost](../build-cost/README.md).

На niri используйте `background-gui`: создайте собственный display командой
`bgui.py start`, откройте read-only watcher через `bgui.py watch SESSION`,
затем запустите скрипт **внутри** этого display:

```sh
python3 /absolute/background-gui/scripts/bgui.py run SESSION --cwd /absolute/Proteus-desktop -- \
  python3 /absolute/Proteus-desktop/examples/performance/native-history/run.py \
  --native /absolute/after/proteus-native \
  --core /absolute/after/proteus \
  --module /absolute/after/proteus-reference-module \
  --seeder /absolute/session_index_fixture \
  --extensions /absolute/Proteus-desktop/build/Proteus/share/extensions \
  --core-source /absolute/Proteus-core \
  --output /absolute/Proteus-desktop/build/performance/native-after \
  --messages 5000 50000 --repeats 3 --settle 5 --idle 5 \
  --bgui-session SESSION --bgui /absolute/background-gui/scripts/bgui.py
```

Повторите для `before` в новом output. Существующие output/case не
перезаписываются. По окончании остановите собственный display через
`bgui.py stop SESSION`. Пользовательские sessions/config не используются.

В output остаются SHA256 executables и исходников стенда, параметры, синтетические journals,
непрерывные `samples.jsonl`, native startup traces, инвентарь окна, screenshot
и `report.json`. После измерения каждый запуск проверяет через SSE точный
session ID, число сообщений по cursor и текст/IDs последних 50 сообщений.
Canonical journal и blobs должны остаться побайтно неизменными.
Измеряются RSS, PSS, SwapPss, private memory, CPU, IO и sampled peak всего
дерева: GUI, backend, modules, extensions. Сам sampler, compositor и observer
в сумму приложения не входят. 100% CPU означает одно логическое ядро.

Readiness разделена на `backend.ready`, `window.create`,
`chat.snapshot.applied`, `chat.transcript.render`. Последняя отметка означает
первый render непустой истории, а не подтверждение presented frame GPU.
Внешнее время наблюдения readiness ограничено сверху интервалом опроса.
Память после settle считается медианой idle samples; CPU — по сумме тиков
за интервал. Peak является нижней оценкой по выборкам. Сумма RSS дважды
считает shared pages; основной показатель — PSS вместе с отдельно показанным
SwapPss. Ошибка readiness, неполный `/proc`, превышение RAM-порога или
изменение journal завершают запуск ошибкой с сохранением частичных samples.

Этот сценарий не запускает model turns и не измеряет десять субагентов:
для этого существует [Core history-scale](../../../../Proteus-core/examples/performance/history-scale/README.md).

Классификация процессов включает Rust-бинарники внутри каталога расширений
и их Node worker. Целевая проверка: `python3 examples/performance/native-history/test_runner.py`.
