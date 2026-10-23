# UI checkpoint — 23 октября 2026

Исходный checkpoint сохранён по просьбе владельца для продолжения дома.
На тот момент portable оставался на `a8e182fc92b7`.

После продолжения пункты 1–4 прошли повторную native-проверку; исправлены
автопрокрутка rich content, адресация SVG errors, hover соседних tooltip и
позиция Show more. Условия и фактические проверки — в заключительном разделе
[ux-review.md](ux-review.md#заключительная-native-проверка-23-октября).
Доставка пункта 5 подтверждается отдельно manifest готового portable и
локальным отчётом `build/verification/ui-2026-10-23/`. Пункт 6 остаётся
отдельным продолжением широкого Zed-аудита.

## Что сохранено

- Общие примитивы Picker, Search, Sortable, Loading, selected menu option и
  divider; публичные capability/SDK, Git/Model и Settings используют их.
- Разделители sidebar/split/колонок рисуют 1 px, имеют невидимый захват 7 px;
  исправлен перенос старого drag-id, подсвечивавший sidebar при resize столбца.
- Повторное нажатие закрывает меню; общий reveal вместо перелёта popup,
  компактные enum/checkbox, whole-row reorder Extensions/Hooks без ручек/стрелок.
- Общие selected/hover, привязанные к элементу подсказки только для обрезанных
  подписей, постоянная задержка каждой новой подсказки, короткий fade.
- MRU/закрытие/восстановление вкладок и сохранение фокуса фоновой вкладки;
  исправлены plain unified multi-file/no-newline diff и Git search navigation.
- Сохранение failed-send drafts вместе с изображениями; фоновые attachments.
- Retained blank tail при collapse инструментов/групп/preview/subagent.
- Activity dock над composer: project, публичные селекторы, все вопросы,
  approvals и план; подробности раскрываются вверх, полоса остаётся у ввода.
- Общая content surface для зарегистрированных fenced languages в ответах;
  независимый процесс json-render, native full-color SVG, fallback/source/copy,
  optional `skills/visualize/SKILL.md` в пакете.
- SDK отвечает запрошенной поверхности раньше скрытых документов; Core
  отдаёт per-path source revision для актуального working diff cache.

## Исходный список продолжения

1. Собрать новый native и проверить последние изменения в реальном окне:
   Search button/inline field в Settings и Git Graph (Ctrl+F, два Esc, narrow);
   hover заметно слабее selected во всех меню/строках и после смены темы;
   каждая новая подсказка ждёт полную задержку, полная подпись не дублируется.
2. Проверить collapse инструментов, цепочек, Show more и subagents возле низа:
   заголовок остаётся под курсором, освобождённое место намеренно остаётся
   пустым и убирается ручной прокруткой/переходом к концу.
3. Проверить activity dock: все активные элементы доступны над вводом;
   ответы двух вопросов не теряются; Enter/Esc/цифры адресуют видимый request;
   свёрнутый approval сначала раскрывается; Esc меню не отклоняет approval;
   strip не уезжает, attachments/send работают и в узком окне.
4. Проверить rich content end-to-end: JSON/SVG, неизвестный язык, незавершённый
   fence, невалидные данные, исходник/Copy, сохранение вкладок/фильтров после
   scroll; изоляция чатов, install/update/disable renderer без пересборки host.
   Optional skill отдельно подключается владельцем, автоматически не ставится.
5. После native-проверки закончить review и собрать portable из чистых
   закреплённых commits, проверить packaged binary/все assets и manifests,
   затем перезапустить рабочее приложение. Не выдавать unit gate за готовый UI.
6. Продолжить широкий Zed-аудит по docs/ux-review.md: поиск Git пока только
   по загруженной истории, cache чужих branches без смены HEAD может устареть;
   большие diff требуют измерения UI latency, tool output ещё не полностью
   использует общий syntax-aware code viewport.

## Проверено на момент checkpoint

- Client: 164 unit tests + 4 client-common, форматирование — PASS.
- Extensions: 84 Rust + 14 Python tests, 106 real wire messages / 11 packages — PASS.
- Core: 18 затронутых Git HTTP tests — PASS.
- Предыдущий development native snapshot подтвердил: divider 1 px и изоляцию
  sidebar, resize по полной высоте, отмену Sortable через Esc, child controls,
  keyboard reorder/задержанный ответ, actual Extensions/Hooks reorder, dirty
  close guard; Settings/Picker selected colors и failed draft/image restore.
- На момент исходного checkpoint последние Search/dock/tail/tooltip/theme/rich
  content после объединения ещё не проходили повторную native-проверку.
  После продолжения: client 165 + 4, Extensions 84 Rust + 14 Python и
  106 wire messages; synthetic native-сценарии перечислены в UX-аудите.

Общие правила: DESIGN.md. Аудит: docs/ux-review.md. Примеры renderer и skill:
соседний Extensions/packages/json-render/README.md и examples/rich-content.md.
Все cargo-команды на ноутбуке — через ~/.local/bin/lowprio, последовательно;
portable — scripts/portable.py --fast с PROTEUS_*_SOURCE на соседние checkout.
