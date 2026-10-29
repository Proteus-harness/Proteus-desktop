# Native content fixtures

Синтетические данные для постоянной проверки Markdown, fenced Mermaid/Pele,
JSON-каталога, выделения и естественной геометрии кода. Внешние модели и реальные
чаты не используются. Показатели в `chat-content.md` — текст fixture, не замер.

Из Proteus-core используйте уже готовый `native_scroll_fixture`;
ради UI-проверки Core не пересобирается:

```sh
target/release/examples/native_scroll_fixture \
  "$PWD/target/verification/native-content/config" \
  "$PWD/target/verification/native-content/workspace" 2 --content \
  /absolute/Proteus-desktop/examples/fixtures/chat-content.md
```

Откройте полученную session в native client с отдельным config и fake-profile.
Проверьте заголовки, таблицу, язык/копирование кода, кириллицу в обеих диаграммах,
JSON, source toggle, выключение/включение провайдера и scroll в самом низу.
`native_scroll_fixture ... 12 --tool-details` проверяет tool tooltip, цепочку
повторных имён и длинный embedded diff. Не пишите journal вручную.

`chat-review.md` сохраняет пример владельца: display LaTeX через `$$`,
кириллица и emoji, Mermaid с обратным ребром, компактная таблица и task-list.
Дополнительно проверьте одинаковый фон кода и номеров строк, отсутствие folding
в чате, inline code, дату после повторного открытия, отступы и настройки шрифтов.
