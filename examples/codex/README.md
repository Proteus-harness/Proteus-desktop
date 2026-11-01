# Синтетический Codex

`mock-app-server.py` — локальный исполняемый app-server для проверки Desktop,
без авторизации, запросов к моделям и сети. Путь задаётся переменной окружения,
отдельного поля выбора executable в настройках пока нет:

```bash
PROTEUS_CODEX_EXECUTABLE="$PWD/examples/codex/mock-app-server.py" \
  ./build/Proteus/proteus-native /path/to/test-project --agent codex
```

Он содержит длинный чат из 600 строк и вложенный чат. Новый запуск без
сохранённых вкладок создаёт отдельный пустой чат.
Чтобы сохранять новые синтетические чаты между запусками, задайте
`PROTEUS_CODEX_FIXTURE_STATE` как путь к JSON-файлу внутри каталога проверки.

Отправьте обычный текст для streaming Markdown, reasoning и вывода команды.
Специальные сообщения: `/approval`, `/question`, `/permissions`, `/slow`
(проверка остановки), `/error`, `/disconnect`. Для изоляции предпочтений Desktop задайте отдельный абсолютный
`XDG_CONFIG_HOME`: fixture не читает настоящие истории, но приложение сохраняет
выбор агента и вкладки.
