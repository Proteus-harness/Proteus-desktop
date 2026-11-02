# Нативный терминал

`proteus-terminal` отделяет PTY и VT engine от GPUI. Desktop получает общий
`Terminal` и неизменяемый снимок видимого `Screen`; рендерер вкладки остаётся
тем же. В одном терминале работает один engine и один scrollback.

Основной engine — `libghostty-vt` из Ghostty; Alacritty оставлен для сравнения
и явного возврата через `PROTEUS_TERMINAL_ENGINE=alacritty`. Ghostty включён
Cargo feature `ghostty-vt`; этот feature включён в обычную сборку Desktop.
`PROTEUS_TERMINAL_ENGINE=ghostty` требует присутствия engine в сборке и не
подменяется другим engine при ошибке. Неизвестное значение — явная ошибка.

Это VT-библиотека, а не весь Ghostty: GPUI рисует существующий `Screen`,
`portable-pty 0.9.0` создаёт shell и PTY, Ghostty разбирает поток и кодирует
клавиши с учётом текущих legacy/Kitty modes. Node и web UI не запускаются.
Присоединение к PTY, принадлежащему агенту, — отдельная граница; замена VT
engine сама по себе его не реализует.

## Происхождение

- [Ghostty](https://github.com/ghostty-org/ghostty), `libghostty-vt`, MIT:
  `83fb95624b0439ee660c2679b43b38c8b63769cd`.
- Готовые Rust bindings `ghostty-vt` / `ghostty-vt-sys`
  `0.8.0+ghostty.1.4.0-main.83fb956`,
  [gpui-cn](https://github.com/prabirshrestha/gpui-cn/tree/e49d909b2ceae040714aa53cd38f97faa4f85cd2/crates/ghostty-vt),
  выбран Apache-2.0 из upstream MIT OR Apache-2.0. Этот fork основан на
  [libghostty-rs](https://github.com/Uzaaft/libghostty-rs).
- Точные опубликованные bindings сохранены в `vendor/ghostty-vt*` с
  исходными manifests и лицензиями. Локальные изменения перечислены в
  `PROTEUS.md`: проверенный Rust 1.95 вместо общего upstream MSRV 1.96 и
  устранение наблюдения за отсутствующим submodule gitlink, которое иначе
  инвалидировало бы каждую повторную Cargo-сборку. VT parser не переписан.

## Потоки, память и завершение

Один actor владеет `!Send` Ghostty handles и PTY fd. UI не разбирает VT и не
ждёт его resize/reflow под общим mutex: под коротким lock копируются команды
и публикуется готовый видимый снимок. Nonblocking poll обслуживает ввод,
вывод и resize. Отключённая от отрисовки вкладка продолжает разбор вывода,
но не готовит непрерывную очередь кадров. Снимки ограничены текущей сеткой;
единственный pending redraw объединяет повторные уведомления.

- Scrollback: 8 МиБ **приблизительного** лимита и 10 000 физических строк.
  Upstream удаляет полные страницы; оба значения могут превышаться на
  гранулярность страниц. Это не жёсткая граница RSS всего процесса.
- Неактивные страницы сжимаются ограниченными incremental steps: начало
  после 250 мс без активности, продолжения через 16 мс. Полный синхронный
  проход по истории в UI не запускается.
- Очередь ввода: 1 МиБ и 64 команды; очередь ответов VT в PTY: 64 КиБ.
  Чтение идёт блоками по 16 КиБ, до 64 КиБ за проход; сырая история вывода
  дополнительно не хранится. Переполнение ввода отклоняется с сообщением,
  shell и текущая команда продолжают работать.
- Видимая сетка ограничена 1000×500. Кадр публикуется не чаще примерно
  60 раз/с и только при наличии разрешения UI на следующий снимок.
- APC ограничен 64 КиБ, Glyph Protocol отключён, запись в clipboard через
  терминальные протоколы запрещена. Графические storage не включаются.

Закрытие вкладки сигналит actor без ожидания на UI. На Linux при запуске
сохраняется идентичность отдельной PTY session; cleanup находит и уже
переподчинённые background jobs этой session. Сигналы идут через pidfd с
проверкой SID/start time до и после открытия handle, поэтому повторное
использование PID не направляет сигнал другому процессу. HUP получает
короткий срок на завершение, затем оставшиеся процессы завершаются и shell
reap-ится. После возвращения из UI event loop приложение вызывает
`shutdown_all()`: общий срок ожидания всех actor ограничен двумя секундами.
Намеренно отделённые в новую OS session daemon-процессы не становятся
универсально управляемыми терминальной библиотекой.

## Сборка и проверки

Zig 0.17.0 нужен при сборке VT library; статически связанный runtime не
требует Zig, toolchain или исходников. `scripts/terminal_toolchain.py`
использует явно заданный `ZIG`, подходящий `zig` из `PATH` либо официальный
архив в `build/tools`. Linux x86_64 archive проверяется SHA256; семь
зависимостей загружаются с закреплённых первичных источников, затем
проверяются исходными Zig package hashes из Ghostty. Кэш сохраняется;
совпадающий receipt позволяет пропустить повторную подготовку. Общий file
lock защищает одновременные run/test/portable до блокировки Cargo.

Обычные Desktop scripts подготавливают toolchain при выбранном Ghostty
feature. Для прямого Cargo:

```bash
python3 scripts/terminal_toolchain.py
ZIG="$PWD/build/tools/zig-x86_64-linux-0.17.0/zig" \
  cargo test -p proteus-terminal --features ghostty-vt --lib
```

Указанный путь относится к автоматически загруженному Linux toolchain;
если используется внешний `ZIG`, сохраняйте его значение. Обход проверки
Rust версии не нужен.

Тесты покрывают Unicode/graphemes и стили, cursor/alternate screen,
query replies, legacy/Kitty keyboard modes, resize/reflow, ограничение
scrollback на 10k→100k строк, bounded input и продолжение работы после
отклонённой вставки, вывод реального PTY, скрытый burst, закрытие shell и
фоновых процессов, в том числе после самостоятельного выхода shell.
Ресурсный A/B сценарий —
[`examples/performance/terminal`](../../examples/performance/terminal/README.md).
