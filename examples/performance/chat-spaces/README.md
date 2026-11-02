# Native: отдельная рабочая область каждого чата

Синтетический HTTP/SSE app-server для проверки Desktop без запуска Core,
backend-модулей, моделей и настоящих профилей. Используется существующий
[MIT transport native-stream](../native-stream/fixture_server.py), его
[лицензия](../native-stream/LICENSE-MIT.fixture) сохраняется в исходном месте.
DTO сверены с `proteus-contracts::app_protocol::{http, live, session}`.

Один сервер слушает случайный порт только `127.0.0.1`, требует случайный Bearer.
Каталог содержит основные чаты **A**, **B** в `workspace/` и **childA** с
`parent_session_id` A, но с другой папкой `child-workspace/`. Каждый чат имеет
свой UUID, root-thread, immutable историю с видимым маркером `CHAT SPACE …`
и отдельный SSE baseline. Одновременные подписки и переподключения не меняют
общее поле текущей сессии. SSE после baseline передаёт только heartbeat;
отправка сообщений и работа агента в этот сценарий не входят.

Из корня Desktop, после сборки нужной версии:

```sh
python3 examples/performance/chat-spaces/run.py \
  --output "$PWD/build/verification/chat-spaces-native-20261102" \
  --native "$PWD/target/debug/proteus-native" \
  --extensions "$PWD/build/Proteus/share/extensions" \
  --timeout 1800
```

`--native` принимает абсолютный путь к готовому Native или portable launcher.
`--extensions` позволяет debug Native использовать готовые расширения portable;
без этого флага действует обычный поиск самого launcher. Ничего не собирается.
Выходная папка должна быть новой: запуск не перетирает чужое состояние.
На niri запускайте эту команду через `background-gui` в отдельной сессии.
Сам runner не взаимодействует с рабочим столом и не проверяет внешний вид.

Все HOME/XDG/Proteus каталоги изолированы внутри output. В preferences записан
канонический `ChatSpaces` с A, но `tabs_workspace` остаётся `null`: временный
`--origin` не сохраняется как постоянный server namespace. Поэтому старт в
attach mode проверяется через `/bootstrap`, всегда возвращающий A. Постоянное
восстановление областей после перезапуска требует штатного spawn namespace и
отдельно проверяется тестами Desktop.

Для одного транспорта без запуска Native используйте `--serve-only` вместо
`--native`. `launch.json` содержит origin, случайный token, переменные окружения
и каталог сессий; его можно использовать для отдельного запуска Native.

Ручной сценарий:

1. Убедиться, что открылся A. Открыть файл `main.txt` и терминал, разложить плитки.
2. Выбрать B: его область должна быть независимой; вернуться в A и проверить
   сохранность плиток, активного документа и терминала.
3. Раскрыть A в боковом списке и открыть childA в плитке области A. История
   должна содержать `CHAT SPACE childA`, а файлы/терминал области — использовать
   корневую папку A. В child-папке лежит другой маркер `child.txt`.
4. Создать новый основной чат. `/new-session` создаёт C, затем `NEW5` и т. д.
   (не более 16 сессий), возвращает `SessionOpened`, не делает платных запросов.
5. Удалить C и проверить, что A/B не изменились. Fixture поддерживает
   `/delete-session` для ровно указанной сессии; каскадное удаление не моделируется.
6. Остановить стенд: `touch OUTPUT/control/stop`. Runner завершит только свою
   process group, закроет сервер и дождётся HTTP handlers. Можно также SIGINT.

Поддерживаются `/sessions`, `/bootstrap`, `/resume`, `/new-session`,
`/delete-session` и scoped `/snapshot`, `/events`, `/config`, `/config/local`,
`/context`, `/sessions/current`, `/workspace/list`, `/workspace/file`,
`/workspace/changes`, `/model/quota`, `/history/tool-state?name=update_plan`.
Badge квоты сообщает отсутствие лимитов, План — отсутствие сохранённого плана.
Другие query keys и имена tool-state по-прежнему отвергаются, а полный путь
запроса записывается до валидации. Files читает только синтетические папки адресованной
сессии; Git status возвращает отсутствие репозитория. Другие запросы считаются ошибкой сценария и записываются в state.
Закрытие SSE при смене области ожидаемо и не считается ошибкой.

Артефакты: `launch.json`, `state.json`, `trace.json`, `native.log`.
`native_verified` всегда остаётся `false`: подключение SSE не доказывает
визуальное поведение. Результаты GUI сохраняйте отдельным отчётом со снимками.

Focused Python-проверки:

```sh
python3 -m unittest discover -s examples/performance/chat-spaces -p 'test_*.py' -v
```
