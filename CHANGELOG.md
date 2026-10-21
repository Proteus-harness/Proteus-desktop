# Изменения

## 0.1.0-alpha.2

Первый согласованный experimental-выпуск четырёх репозиториев Proteus.

- Общая Linux x86_64 portable-поставка включает Desktop, Core, reference-модуль,
  DCP, профили и процессные расширения клиента. Ревизии исходников закреплены.
- Experimental Rust SDK позволяет написать внешний процессный tool-модуль
  на текущих wire v3 и tool/v6. Самостоятельный starter проверяется без
  импорта Core или reference implementations.
- GitHub Actions запускаются вручную; тег `v*` в Desktop проверяет выбранный
  комплект, собирает архивы и публикует prerelease. Обычные push и PR не
  запускают сборки.
- В CLI и меню Desktop показываются версия и commit; `proteus-native --version`
  работает без открытия окна.
- Релиз содержит контрольные суммы, ревизии всех исходников и лицензии.
- Для process components и stdio MCP доступна явно включаемая Linux-изоляция
  через bubblewrap. Она покрывает только настроенные пути запуска.

Форматы SDK, config, API, wire и истории остаются experimental и могут
меняться между alpha-выпусками. Поддержка Windows/macOS, общий sandbox всех
подсистем и установка процессных модулей из каталога в этот выпуск не входят.

Для DCP нужен системный Node.js 22+, для расширений клиента — Python 3.
Режим `restricted` требует bubblewrap и доступные user namespaces.
