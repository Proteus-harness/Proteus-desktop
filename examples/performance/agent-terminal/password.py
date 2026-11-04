#!/usr/bin/env python3
"""Synthetic password prompt; never invokes sudo or a package manager."""
import getpass
import time

print("Проверка терминала команды агента. Установка пакетов не выполняется.", flush=True)
print("Введите любой ВЫДУМАННЫЙ пароль. Его текст не печатается.", flush=True)
password = getpass.getpass("[demo sudo] Пароль: ")
if password:
    print("\nPASSWORD_ACCEPTED · скрытый ввод получен", flush=True)
else:
    print("\nПустой ввод получен", flush=True)
del password
for step in range(1, 4):
    print(f"Проверка {step}/3…", flush=True)
    time.sleep(0.3)
print("\033[32mTERMINAL_DEMO_DONE\033[0m", flush=True)
