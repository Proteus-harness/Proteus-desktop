# Терминал: одинаковый PTY replay для Alacritty и Ghostty

Fixture находится в `crates/terminal/examples/replay.rs`, runner — рядом.
Один release binary содержит оба engine; каждый запуск создаёт только один
терминал. `SHELL=/bin/sh`, сетка 100×30; awk пишет 10k или 100k строк с ASCII,
кириллицей, широкими символами, emoji и ANSI цветом. Конец определяется по
полной строке маркера, а не по её появлению внутри эха отправленной команды.

Сначала подготовьте toolchain и соберите fixture. Во время замера Cargo
не запускается:

```bash
python3 scripts/terminal_toolchain.py
ZIG="$PWD/build/tools/zig-x86_64-linux-0.17.0/zig" \
  cargo build -p proteus-terminal --features ghostty-vt --example replay --release
python3 examples/performance/terminal/run.py \
  --output build/verification/terminal-ab/report.json --runs 3
```

Не запускайте параллельную компиляцию. Runner чередует engine в каждом из
трёх повторов, сохраняет SHA256 binary, fingerprint terminal sources, grid,
workload и все исходные результаты. Сам процесс читает RSS/PSS сразу после
маркера и после одинаковой секунды покоя: у Ghostty incremental compression
начинается через 250 мс. Runner дополнительно опрашивает всё дерево
процессов каждые 10 мс и считает суммарные RSS/PSS/Swap peaks и CPU time.
Sampled peak может пропустить более короткий пик; self VmHWM сохраняется
отдельно как сообщение Linux `/proc`, без подмены sampled tree peak.

После измерения fixture закрывает терминал и ждёт освобождения PTY. Эти
показатели относятся к engine/PTY process и его shell/awk. GPUI, Codex и
Core в fixture не запускаются; экономия PSS здесь не равна экономии всей
desktop-сессии. Существующий Alacritty хранит до 10k строк, Ghostty имеет
также приблизительный byte cap 8 МиБ и page compression. Политики retention
указаны явно и не являются доказательством совпадения всей скрытой истории.

## Проверка 2026-11-02

17 native headless tests прошли на Rust 1.95.0 без обхода MSRV; отдельная
GPUI проверка готового приложения остаётся самостоятельной проверкой.
A/B: три повтора каждой пары в одном release binary, компиляция остановлена.
Медианы self PSS в КиБ:

| Строк вывода | Alacritty сразу / через 1 с | Ghostty сразу / через 1 с |
|---|---:|---:|
| 10 000 | 26 680 / 26 680 | 10 687 / 3 819 |
| 100 000 | 26 688 / 26 688 | 11 015 / 3 715 |

Медианы sampled tree peak PSS: 29 718→12 439 КиБ на 10k и
29 757→14 033 КиБ на 100k; Swap — 0. Replay занимал 45,7/41,2 мс на 10k
и 251,4/252,8 мс на 100k (Alacritty/Ghostty). Преимущество в расходе памяти
видно на этом сценарии; скорость 100k здесь сопоставима. Это не измерение
производительности самого агента или полного приложения.

Исходный локальный отчёт:
`build/verification/ghostty-terminal-20261102/ab-report.json`.
Он содержит SHA256 измеренного binary и все 12 запусков. Этот файл —
локальная verification artifact; runner и fixture хранятся в Git.
