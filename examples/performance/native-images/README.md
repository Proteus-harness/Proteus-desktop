# Память изображений в Native

Постоянный синтетический стенд для сравнения **готовых** версий Desktop.
Cargo, Core, модели и реальные истории не используются. HTTP/SSE transport
импортируется из `../native-stream/fixture_server.py` с его MIT-лицензией;
запуск, SHA-256 исполняемого файла и остановка собственной группы процессов
используют helpers того же стенда. Linux process sampler импортируется из
соседнего `Proteus-core/examples/performance/common/proc_sampler.py` без сборки.

По умолчанию создаются 16 различных детерминированных RGBA PNG 2048×1024:
по две картинки в восьми пользовательских сообщениях, после каждой пары —
текстовый маркер ассистента. Общий декодированный объём — 128 МиБ. PNG пишутся
построчно через стандартные `zlib`/`struct`; Python не держит декодированный
корпус. HTTP читает только запрошенный PNG. SHA-256, размеры и имена записаны
в `corpus.json`. Пути различаются между запусками; хеши PNG при одинаковых
параметрах совпадают.

Запускать внутри отдельной background-gui сессии согласно AGENTS.md:

```sh
python3 examples/performance/native-images/run.py \
  --native "$PWD/build/Proteus/proteus-native" \
  --output "$PWD/build/verification/native-images-before" \
  --image-delay-ms 50
```

Стенд не управляет GUI и не открывает watcher. Его запускающий агент делает
это отдельно через `background-gui`; не запускайте второй экземпляр на
пользовательском дисплее ради замеров. Выходная папка должна быть новой и
находиться внутри `Desktop/build/verification`. HOME, XDG, profile, workspace
и extensions изолированы. По умолчанию расширения отключены; `--extensions`
разрешает явно передать одну и ту же готовую директорию обоим сравнениям.
Loopback-сервер получает случайный Bearer, а токен скрыт в `launch.json` и
process samples. Настоящие credentials и профили не читаются.

После `READY` приложение стоит на обычном начальном дне чата. Для сравнения:

1. Дождаться одинакового idle, снять скрин и отметить исходный замер.
2. Прокрутить весь чат вверх, затем вниз, позволяя каждому изображению
   появиться. Число реально запрошенных ID проверять в `state.json`.
3. После одинаковой паузы снять замер удерживаемых изображений.
4. Послать замену истории пустым snapshot и после idle снять замер освобождения.
5. Остановить стенд. Повторить на другой готовой версии с теми же параметрами,
   дисплеем, размером окна, renderer и последовательностью действий.

Команды контроля — обычные файлы в созданной output-папке:

```sh
touch build/verification/native-images-before/control/mark-bottom
echo scroll-all > build/verification/native-images-before/control/phase
touch build/verification/native-images-before/control/mark-loaded
touch build/verification/native-images-before/control/replace
touch build/verification/native-images-before/control/mark-released
touch build/verification/native-images-before/control/stop
```

`mark-NAME` записывает отдельный именованный sample; каждый NAME принимается
один раз. `phase` задаёт подпись последующих samples. Замена разрешена один раз,
после подключения SSE: immutable initial snapshot seq=0 → empty snapshot seq=1
с тем же stream/session ID. Другие runtime-события и произвольные snapshot
отвергаются. Это тест удаления ссылок из истории, а не модель обычного ответа
агента. Snapshot-only fixture специально не проверяет delta/replay correctness.

Доступны `--images` (чётное 2–32), `--width`/`--height` (32–2048), совокупный
decoded RGBA ≤256 МиБ, каждый encoded PNG ≤5 МиБ. `--image-delay-ms` (0–250,
по умолчанию 50) добавляет одинаковую искусственную задержку в каждый image
fetch, чтобы параллельные запросы успевали пересечься на loopback. Это не
измерение сетевой latency; параметр должен совпадать в обеих версиях.
Таймаут — 10–7200 секунд, по умолчанию 1200. SIGINT/SIGTERM и `control/stop`
останавливают только Native process group, созданную этим запуском, и сервер.

Результаты:

- `samples.jsonl`: process tree от PID Native каждые ≈100 мс, PSS/RSS/CPU/I/O,
  фазовые отметки; fixture Python, observer и compositor в дерево не входят.
- `state.json`: текущая стадия, PID, provenance, image fetch counts, максимум
  одновременно активных запросов и именные samples.
- `trace.json`: bounded transport trace, порядок snapshots/requests.
- `report.json`: итог, ошибки fixture, tree totals и sampled peak PSS/RSS.
- `native.log`, `corpus.json`, `launch.json`: воспроизводимость входов/запуска.

Сумма decoder allocations не равна PSS; PSS не включает всю память GPU.
Искусственная история и один проход не доказывают отсутствие утечек во всём
приложении. Сам `native_verified: false` означает, что runner не делает
визуальную приёмку: скрин и ручной/native UI результат фиксируются отдельно.

## Границы миниатюр

Отправленные изображения используют общий для всех чатов LRU: не более
64 миниатюр и 16×1024² пикселей по всем кадрам (64 МиБ BGRA). Строки хранят
слабые ссылки; удаление последнего владельца очищает запись и откладывает
удаление её GPU atlas tiles до безопасного UI-такта. GPUI asset cache с
полноразмерными декодированными оригиналами в этом пути не используется.
Вытесненная видимая миниатюра может загрузиться снова.

Вся цепочка HTTP → decode → принятие результата занимает один из двух общих
слотов. Очередь содержит только метаданные, максимум 512 запросов; переполнение
откладывает загрузку до освобождения места, не превращается в ошибку картинки. Тайм-аут
запроса изображения, включая тело ответа, — 30 секунд; SSE он не затрагивает.
Исходные PNG/JPEG/WebP/GIF ограничены 5 МиБ encoded, 32×1024² пикселей и
128 МиБ декодированных данных, стороной не более 32768. Это admission bounds,
а не доказанный верхний предел PSS: decoder buffers и две активные задачи
потребляют дополнительную память.

Статическая миниатюра — центрированный квадрат 96 логических пикселей с
учётом масштаба приложения и дисплея, максимум 384×384; маленький оригинал
не увеличивается. EXIF orientation применяется до обрезки. GIF/WebP сохраняет все
кадры и задержки до 256 кадров включительно, а совокупный размер его
миниатюр ограничен 4×1024² пикселей уменьшением разрешения. Более длинная анимация
явно отклоняется, не превращается молча в статичную картинку. Ошибка или
превышение лимита видны по значку и подсказке. Composer attachments — другой
путь, этот замер их расход не проверяет.

Проверка корпуса и wire/lifecycle без GUI:

```sh
python3 -m unittest discover -s examples/performance/native-images -p 'test_*.py' -v
```

Тесты проверяют CRC/структуру/размеры PNG, различие хешей, corpus budgets,
binary response byte equality, auth/query/ID rejection, response size limit,
immutable ImageRef, строгую смену snapshot и изоляцию конфигурации.
