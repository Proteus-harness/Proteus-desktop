# Native Terminal: выключение, перезапуск и повторное включение

Постоянный функциональный стенд для **готового** Desktop и готового пакета
Terminal. Cargo, Core, модели, настоящие чаты и пользовательский профиль не
используются. HTTP/SSE взят из MIT-стенда `../native-stream`; здесь добавлены
строгие scoped endpoints `/addons`, `/config/builder`, `/config/history` с
полными DTO закреплённого `proteus-contracts`. Это пустой read-only профиль
агента; настройки клиентских расширений остаются рабочими и локальными.
Неизвестный endpoint, неправильная сессия или неожиданная запись завершают
стенд ошибкой, а не маскируются универсальным пустым ответом.

Запускайте в отдельном дисплее через `background-gui`, не на основном рабочем
столе. Путь `--terminal` указывает на каталог **одного пакета**, содержащий
`extension.json`, а не на общий каталог расширений. Стенд копирует пакет в
свой профиль; исходный portable не изменяется.

```sh
python3 examples/performance/native-terminal/run.py \
  --native "$PWD/build/Proteus/proteus-native" \
  --terminal "$PWD/build/Proteus/share/extensions/terminal" \
  --output "$PWD/build/verification/terminal-lifecycle-after"
```

Реальный путь установленного пакета зависит от portable; сначала найдите
его `extension.json`. `--output` должен быть новым каталогом внутри
`build/verification`. По умолчанию deadline — 30 минут, максимум два часа.
`state.json` и `trace.json` обновляются в ходе запуска, `report.json` пишется
при завершении; Native ELF, файлы пакета и источники стенда имеют SHA-256.
`READY` подтверждает подключение HTTP/SSE, **не появление терминала**.

В интерфейсе откройте Terminal через меню `+` вкладок. В нём выполните:

```sh
printf '%s\n' "$$" > ../control/shell-pid
printf 'TERMINAL_OWNER=%s\n' "$$"
```

Сохраните скриншот с видимым маркером. Рабочая папка оболочки — только
синтетический `OUTPUT/workspace`, поэтому относительный путь ведёт в её
`control`. Затем запомните PID **вместе с Linux start ticks**:

```sh
python3 examples/performance/native-terminal/check.py \
  --output "$PWD/build/verification/terminal-lifecycle-after" \
  --label initial --remember --expect alive
```

Проверки поведения выполняются отдельно, пока Native работает:

1. Откройте Settings → Extensions → Terminal и выключите расширение.
   `check.py --label initial --expect dead` должен пройти. На старом Native
   эта проверка воспроизводит удержание PTY. Ошибка проверки намеренно не
   останавливает стенд: можно сохранить остальные свидетельства.
2. Включите Terminal повторно, откройте вкладку и снова выполните команды
   маркера. `--label reenabled --remember --expect alive` фиксирует новую
   оболочку; PID/start ticks должны отличаться от `shell-initial.json`.
3. Для проверки перезапуска runtime завершите **только принадлежащий этому
   стенду процесс расширения Terminal**, а не Native и не PTY shell. Нажмите
   «Повторить» в его представлении. Маркер должен сохраниться и
   `--label reenabled --expect alive` — пройти с прежними PID/start ticks.
   Перезапуск runtime сохраняет терминал, выключение/удаление — освобождает.

`check.py` проверяет родство с Native перед сохранением и при проверке живой
оболочки, не посылает сигналов и пишет отдельный `proof-*.json` с результатом.
Статистика памяти здесь не собирается: это проверка жизненного цикла, не
доказательство экономии RSS/PSS или отзывчивости интерфейса.

Завершение:

```sh
touch build/verification/terminal-lifecycle-after/control/stop
```

Runner завершает свою группу Native, затем Linux subreaper останавливает и
забирает только усыновлённых им потомков (PTY/процессы расширения могут иметь
отдельные process groups). Неожиданный выход Native, даже с кодом 0, считается
ошибкой; SIGTERM runner и `control/stop` — обычное завершение. Прочие процессы
пользователя не затрагиваются.

```sh
python3 -m unittest discover -s examples/performance/native-terminal -p 'test_*.py' -v
```

Тесты используют только синтетические HTTP-запросы и отдельный поддельный
Native; проверяют DTO, приватный профиль, копирование пакета, scope/auth,
неожиданный выход и очистку отдельной process group, пережившей Native.
Они не заменяют ручную проверку настоящего Terminal в Native.
