# Выборочная сборка Desktop

Правка интерфейса собирает только Desktop. Core, backend-модули и стандартные
расширения берутся из готовой portable-папки целиком. Отсутствие готового
компонента означает ошибку с подсказкой; скрипты не расширяют сборку автоматически.
Компиляция `proteus-contracts` как зависимости клиента не является сборкой
исполняемого сервера `proteus-core`.

## Выбор компонентов

Команды выполняются из Proteus-desktop. База по умолчанию — `build/Proteus`;
`--base /absolute/ready/Proteus` выбирает другую **полную готовую поставку**.
Результат всегда устанавливается в `build/Proteus`, а не поверх внешнего `--base`.

| Изменение | Команда |
|---|---|
| Интерфейс Desktop | `python3 scripts/portable.py --no-archive` |
| Desktop и процессный протокол расширений | `python3 scripts/portable.py --components desktop extensions --no-archive` |
| Набор расширений | `python3 scripts/portable.py --components extensions --no-archive` |
| Сервер Core | `python3 scripts/portable.py --components core --no-archive` |
| Backend-модули | `python3 scripts/portable.py --components modules --no-archive` |
| Явный bootstrap всего стека | `python3 scripts/portable.py --full --no-archive` |
| Копирование готовой поставки без сборки | `python3 scripts/portable.py --reuse-binaries --base /absolute/ready/Proteus --no-archive` |

`--components` принимает любое нужное сочетание `desktop core modules extensions`.
Он несовместим с `--full` и `--reuse-binaries`. Обычный запуск без `--no-archive`
дополнительно создаёт tar.gz. Профиль по умолчанию — обычный `release`;
`--fast` и `--reuse-hosts` удалены. `--debug` предназначен для отладки, а
измерения плавности и памяти выполняются на release. Расширения упаковщик
собирает в release и при выборе `--debug` для остальных компонентов.

Перед выполнением можно посмотреть выбор без Cargo, клонирования исходников
и изменения готовой поставки:

```sh
python3 scripts/portable.py --plan
python3 scripts/portable.py --components desktop extensions --base /absolute/ready/Proteus --plan
```

`--plan` показывает только намерение, не подтверждает полноту базы или
совместимость протоколов. Реальная выборочная сборка сначала проверяет базу.
Если протокол выбранного Desktop не совпадает с готовыми расширениями,
выберите `--components desktop extensions` явно. Скрипт сам их не пересобирает.

## Граница готового компонента

Готовый Core переносится вместе с профилями, модули — со своими исполняемыми
файлами и assets, расширения — целыми пакетами, Desktop — с launcher и значками.
Не смешивайте бинарники из произвольных `target` с assets другого checkout.
`--reuse-binaries` копирует целую базу; изменённые исходники расширений при этом
не упаковываются. Этот режим не подтверждает свежесть исходников.

`share/release-sources.json` сохраняет provenance повторно используемых
компонентов. Выборочная сборка обновляет записи только выбранных исходников,
записывает `build_components` и SHA256 поставляемых бинарников. Подготовка идёт
в отдельной папке; прежняя поставка заменяется после успешной сборки и проверки.
Ошибка сборки оставляет рабочую базу на месте.

Разрешение исходников выполняется только для выбранных компонентов:
`PROTEUS_HARNESS_SOURCE`, `PROTEUS_MODULES_SOURCE`, `PROTEUS_EXTENSIONS_SOURCE`,
соседние checkout или закреплённые revisions из `repositories.json` в
`build/sources`. При выборе модулей дополнительно читаются исходники Core для
hook SDK; сервер Core это не собирает. Для обычной правки UI не нужны pull всех
репозиториев, сборка Core или полный межрепозиторный gate.

## Запуск и проверки

```sh
python3 scripts/run.py /path/to/workspace --config PROFILE
python3 scripts/run.py --base /absolute/ready/Proteus /path/to/workspace
python3 scripts/run.py --build-extensions /path/to/workspace
```

`run.py` собирает dev-клиент через Cargo и использует готовый backend, модули и
расширения из базы. `PROTEUS_BIN` и `PROTEUS_EXTENSIONS_DIR` могут задать готовые
компоненты явно. Только `--build-extensions` запускает упаковщик расширений.
Для подключения через `--origin` локальный backend и его модули не требуются.
Аргументы после `--` передаются приложению без обработки runner.

```sh
python3 scripts/test.py                    # только Desktop workspace
python3 scripts/test.py -p proteus-diff --lib
python3 scripts/test.py -p proteus-tabs --lib
python3 scripts/test.py --extensions       # wire-проверка готового набора
python3 scripts/test.py --build-extensions # явная сборка набора и wire-проверка
python3 -m unittest discover -s scripts -p 'test_release*.py'
```

Обычный `test.py` не готовит расширения и не включает их межрепозиторную
проверку. `--extensions` проверяет готовые бинарники в `packages/<id>/bin`
разрешённого checkout расширений; это не выбор portable через `--base`.
`--build-extensions` сначала готовит эти бинарники. Флаги runner действуют
до разделителя `--`, после него находятся аргументы libtest.

Для документации и сборочных скриптов достаточно применимых целевых
Python-тестов, проверки ссылок и `git diff --check`; сборка Cargo или portable
только ради такой правки не требуется. Поведенческие UI-правки требуют
соответствующих Rust-проверок и отдельного запуска нативного окна.

Для одного расширения выполните в его репозитории
`python3 package.py --package <id>` и установите ZIP через менеджер расширений.
Один backend-модуль обновляется его собственной командой
`python3 package.py --package <id> --install /absolute/ready/Proteus`.
Изменение общего SDK/контракта требует проверки затронутых потребителей.

Полный релизный CI выбирает сборку явно через `portable.py --full`;
Desktop release gate явно использует `test.py --build-extensions`.
Локальная команда по умолчанию не становится полным релизом из-за правил CI.

## Корпуса и измерения

Используйте готовые синтетические чаты или готовый seeder. Не компилируйте
Core ради UI-fixture. Целевая сборка генератора нужна при изменении его кода
или формата истории и необходимости нового корпуса, либо по явному поручению
получить новую fixture таким способом. Постоянные тесты и стенды хранятся в
проекте, а не создаются заново в `/tmp`.

[Native history](examples/performance/native-history/README.md) сравнивает
запуск, повторный запуск, RSS/PSS и CPU настоящего приложения на 5 000/50 000
сообщений с готовыми runtime-компонентами.
[Build cost](examples/performance/build-cost/README.md) измеряет время,
CPU, память дерева сборки, размер поставки и граф зависимостей только выбранных
компонентов:

```sh
python3 examples/performance/build-cost/run.py --output "$PWD/build/performance/build-ui" --jobs 2 --runs 2
python3 examples/performance/build-cost/run.py --output "$PWD/build/performance/build-ui-extensions" --components desktop extensions --jobs 2 --runs 2
```

Git-штамп Desktop отслеживает HEAD и текущую ссылку, в том числе после
`git pack-refs` и в linked worktree. Если loose ref отсутствует, build script
наблюдает существующий ближайший каталог внутри `refs` и существующее
хранилище ссылок, а не отсутствующий файл. Изменение соседней ссылки в этом
каталоге или общего `packed-refs` может консервативно вызвать пересборку;
неизменный checkout не должен пересобираться лишь из-за packed ref.
Штамп остаётся short HEAD: отдельные tags и dirty-файлы не добавляют version
или dirty suffix. Release-профиль, LTO и выбор компонентов не меняются.

Регрессии этой границы — `scripts/test_release_build_commit.py`: маленький
offline Cargo fixture без GPUI/Core проверяет повторную release-сборку,
переходы loose/packed, nested branch, detached HEAD и linked worktree.
Это проверка freshness и штампа, не замер скорости приложения.

Сохраняйте `target` и кэши. Указывайте компоненты, состояние кэша и фактические
метрики; повторная сборка с существующим кэшем не называется холодной. Не
запускайте runtime-бенчмарки одновременно со сборкой: нагрузка искажает время.
