# Перенос нативных полей между окнами

```sh
cargo build --release --example input_rehost
target/release/examples/input_rehost --verify build/verification/input-rehost.json
```

Бинарник запускается через `background-gui` в собственном display. Он создаёт
два настоящих окна и переносит одни и те же Entity Input, Textarea и Editor.
Явный `rehost` не вызывается: проверяется общий автоматический механизм движка.

Проверяются сохранение текста/выделения, Undo/Redo после переноса, ровно одно
событие Focus/Blur в новом окне для каждого поля, закрытие исходного окна и
отсутствие уведомлений от blink timer в течение 1600 ms после blur. Успех
подтверждает JSON со `status: passed`, а не наличие процесса или окна.

Проверка использует программный focus/blur и dispatch стандартных Undo/Redo
в реальном окне. Она не заменяет ручную проверку IME, меню и перетаскивания
конкретной вкладки приложения.
