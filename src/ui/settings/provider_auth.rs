//! Module-owned sign-in commands for a verified local provider executable.
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::{SettingsView, widgets::hashed_id};
use crate::ui::kit::ButtonLabel as _;
use crate::{
    catalog::connections,
    settings::catalog::PageId,
    ui::{kit, prefs},
};

#[derive(Clone)]
struct Connection {
    command: Vec<String>,
    auth: connections::Authentication,
    credential_file: Option<String>,
    cwd: PathBuf,
}

struct Entry {
    connection: Connection,
    authenticated: bool,
    busy: bool,
    cancel: Arc<AtomicBool>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(super) struct State {
    revision: Option<u64>,
    generation: u64,
    entries: HashMap<String, Entry>,
}

impl Drop for State {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            entry.cancel.store(true, Ordering::Release);
        }
    }
}

impl SettingsView {
    pub(super) fn prepare_provider_auth(&mut self, cx: &mut Context<Self>) {
        if self.page != PageId::Providers
            || self.provider_auth.revision == Some(self.profile_revision)
        {
            return;
        }
        let Some(profile) = &self.context.profile else {
            return;
        };
        let Some(cwd) = self
            .workspace
            .read_with(cx, |workspace, _| {
                workspace.provider_auth_workspace(profile.api.connection_id())
            })
            .ok()
            .flatten()
        else {
            return;
        };
        let Some(snapshot) = self.snapshot().cloned() else {
            return;
        };
        for entry in self.provider_auth.entries.values() {
            entry.cancel.store(true, Ordering::Release);
        }
        self.provider_auth.entries.clear();
        self.provider_auth.revision = Some(self.profile_revision);
        self.provider_auth.generation = self.provider_auth.generation.wrapping_add(1);
        let generation = self.provider_auth.generation;
        let installed = prefs::current(cx).catalogs.installed.clone();
        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_executor()
                .spawn(async move {
                    let discovery = connections::discover(&installed);
                    let mut result = HashMap::new();
                    for plugin in &snapshot.plugins {
                        let Some(program) = resolve_program(&plugin.command) else {
                            continue;
                        };
                        for export in plugin
                            .exports
                            .iter()
                            .filter(|export| export.slot == "model")
                        {
                            let config = snapshot
                                .module_config
                                .get("model")
                                .and_then(|config| config.get(&export.id));
                            let Some(config) = config.and_then(Value::as_object) else {
                                continue;
                            };
                            let Some(choice) = discovery.choices.iter().find(|choice| {
                                choice.template.auth.is_some()
                                    && resolve_program(&choice.command[0]).as_ref()
                                        == Some(&program)
                                    && choice.template.module_config.as_object().is_some_and(
                                        |expected| {
                                            expected
                                                .iter()
                                                .all(|(key, value)| config.get(key) == Some(value))
                                        },
                                    )
                            }) else {
                                continue;
                            };
                            let auth = choice.template.auth.clone().unwrap();
                            let credential_file = match config.get(&auth.credential_file_key) {
                                Some(Value::String(path)) if expand_path(path).is_ok() => {
                                    Some(path.clone())
                                }
                                Some(_) => continue,
                                None => None,
                            };
                            let connection = Connection {
                                command: choice.command.clone(),
                                credential_file,
                                auth,
                                cwd: cwd.clone(),
                            };
                            let cancel = Arc::new(AtomicBool::new(false));
                            let authenticated =
                                run(&connection, "status", &cancel).unwrap_or(false);
                            result.insert(
                                export.id.clone(),
                                Entry {
                                    connection,
                                    authenticated,
                                    busy: false,
                                    cancel,
                                },
                            );
                        }
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.provider_auth.generation == generation {
                    this.provider_auth.entries = entries;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn provider_auth_action(&self, id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let entry = self.provider_auth.entries.get(id)?;
        let changed =
            self.draft.module_config("model", id) != self.baseline.module_config("model", id);
        let id = id.to_owned();
        let label = if entry.busy {
            "Отменить вход"
        } else if entry.authenticated {
            "Войти заново"
        } else {
            "Войти"
        };
        let action = kit::button(hashed_id("provider-auth", &id))
            .faded_label(label)
            .quiet(cx)
            .xsmall()
            .disabled(changed)
            .on_click(cx.listener(move |this, _, _, cx| this.sign_in_provider(&id, cx)));
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .when(entry.authenticated && !entry.busy, |line| {
                    line.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Вход выполнен"),
                    )
                })
                .child(action)
                .into_any_element(),
        )
    }

    fn sign_in_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.provider_auth.entries.get_mut(id) else {
            return;
        };
        if entry.busy {
            entry.cancel.store(true, Ordering::Release);
            return;
        }
        entry.busy = true;
        entry.cancel = Arc::new(AtomicBool::new(false));
        let cancel = entry.cancel.clone();
        let connection = entry.connection.clone();
        let id = id.to_owned();
        let generation = self.provider_auth.generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { run(&connection, "login", &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.provider_auth.generation != generation {
                    return;
                }
                if let Some(entry) = this.provider_auth.entries.get_mut(&id) {
                    entry.busy = false;
                    match result {
                        Ok(authenticated) => {
                            entry.authenticated = authenticated;
                            this.show_feedback("Вход выполнен", false, cx);
                        }
                        Err(error) => this.show_feedback(error.to_string(), true, cx),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn resolve_program(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() {
        return path.canonicalize().ok();
    }
    if command.contains('/') {
        return None;
    }
    let backend = crate::backend::proteus_binary();
    let binary_dir = backend.parent().filter(|path| !path.as_os_str().is_empty());
    binary_dir
        .and_then(|dir| dir.join(command).canonicalize().ok())
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .find_map(|dir| dir.join(command).canonicalize().ok())
        })
}

fn run(connection: &Connection, action: &str, cancel: &AtomicBool) -> Result<bool> {
    ensure!(!cancel.load(Ordering::Acquire), "Вход отменён");
    let (program, arguments) = connection
        .command
        .split_first()
        .context("У модуля нет команды входа")?;
    let output_path =
        std::env::temp_dir().join(format!("proteus-provider-auth-{}", uuid::Uuid::new_v4()));
    let mut output = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output_path)?;
    std::fs::remove_file(output_path)?;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .args(&connection.auth.command)
        .current_dir(&connection.cwd);
    if let Some(path) = &connection.credential_file {
        command.arg("--auth-file").arg(expand_path(path)?);
    }
    command
        .arg("--json")
        .arg(action)
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::null());
    let mut child = OwnedChild(
        command
            .spawn()
            .context("Не удалось запустить вход провайдера")?,
    );
    let timeout = if action == "login" {
        Duration::from_secs(15 * 60 + 5)
    } else {
        Duration::from_secs(10)
    };
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) || start.elapsed() > timeout {
            let _ = child.0.kill();
            let _ = child.0.wait();
            bail!(if cancel.load(Ordering::Acquire) {
                "Вход отменён"
            } else {
                "Истекло время входа"
            });
        }
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "Не удалось войти. Повторите вход через провайдера."
            );
            break;
        }
        ensure!(
            output.metadata()?.len() <= 64 * 1024,
            "Ответ авторизации слишком большой"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    if action == "login" {
        let authenticated = run(connection, "status", cancel)?;
        ensure!(authenticated, "Вход не завершён");
        return Ok(true);
    }
    output.seek(SeekFrom::Start(0))?;
    let mut json = String::new();
    output.take(64 * 1024 + 1).read_to_string(&mut json)?;
    let response: Value =
        serde_json::from_str(&json).context("Модуль не вернул состояние входа")?;
    response
        .get("authenticated")
        .and_then(Value::as_bool)
        .context("Модуль не вернул состояние входа")
}

struct OwnedChild(std::process::Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn expand_path(path: &str) -> Result<PathBuf> {
    ensure!(
        !path.is_empty() && !path.chars().any(char::is_control),
        "Неверный путь авторизации"
    );
    for prefix in ["~", "$HOME", "${HOME}"] {
        if path == prefix {
            return Ok(PathBuf::from(
                std::env::var_os("HOME").context("HOME не задан")?,
            ));
        }
    }
    for prefix in ["~/", "$HOME/", "${HOME}/"] {
        if let Some(relative) = path.strip_prefix(prefix) {
            return Ok(
                PathBuf::from(std::env::var_os("HOME").context("HOME не задан")?).join(relative),
            );
        }
    }
    let path = PathBuf::from(path);
    ensure!(
        path.is_absolute(),
        "Для входа укажите абсолютный путь файла авторизации"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[::core::prelude::v1::test]
    fn module_login_uses_its_metadata_and_exact_configured_file_then_checks_status() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification")
            .join(format!("provider-auth-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("module.py");
        let transcript = root.join("arguments");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json, pathlib, sys
with pathlib.Path('arguments').open('a') as out:
    out.write(json.dumps(sys.argv[1:]) + '\n')
print(json.dumps({'authenticated': True}))
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let connection = Connection {
            command: vec![script.to_str().unwrap().into()],
            auth: connections::Authentication {
                command: vec!["custom-auth".into()],
                credential_file_key: "credentials".into(),
            },
            credential_file: Some(root.join("credentials.json").to_str().unwrap().into()),
            cwd: root.clone(),
        };
        assert!(run(&connection, "login", &AtomicBool::new(false)).unwrap());
        let records: Vec<Value> = std::fs::read_to_string(transcript)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            records,
            vec![
                serde_json::json!([
                    "custom-auth",
                    "--auth-file",
                    root.join("credentials.json"),
                    "--json",
                    "login"
                ]),
                serde_json::json!([
                    "custom-auth",
                    "--auth-file",
                    root.join("credentials.json"),
                    "--json",
                    "status"
                ]),
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[::core::prelude::v1::test]
    fn cancellation_never_starts_the_module() {
        let connection = Connection {
            command: vec!["/must-not-run".into()],
            auth: connections::Authentication {
                command: vec!["auth".into()],
                credential_file_key: "file".into(),
            },
            credential_file: None,
            cwd: PathBuf::from("/"),
        };
        assert!(
            run(&connection, "login", &AtomicBool::new(true))
                .unwrap_err()
                .to_string()
                .contains("отменён")
        );
    }
}
