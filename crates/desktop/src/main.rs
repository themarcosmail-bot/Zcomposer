use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::{Value, json};
use tauri::{Manager, Window, WindowEvent};
use zcomposer_core::Store;

struct Project {
    root: PathBuf,
    lifecycle: Mutex<WindowLifecycle>,
    draft: Arc<Mutex<Option<(i64, String)>>>,
}

#[derive(Default)]
struct WindowLifecycle {
    active_turn: Option<Arc<AtomicBool>>,
    closing: bool,
}

impl WindowLifecycle {
    fn start_turn(&mut self) -> Result<Arc<AtomicBool>, &'static str> {
        if self.closing {
            return Err("A janela está sendo fechada.");
        }
        if self.active_turn.is_some() {
            return Err("Já existe um turno em execução nesta janela.");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_turn = Some(Arc::clone(&cancel));
        Ok(cancel)
    }

    fn interrupt_turn(&self) -> bool {
        if let Some(cancel) = &self.active_turn {
            cancel.store(true, Ordering::Release);
            return true;
        }
        false
    }

    fn request_close(&mut self) -> bool {
        self.closing = true;
        self.interrupt_turn()
    }

    fn finish_turn(&mut self) -> bool {
        self.active_turn = None;
        self.closing
    }
}

impl Project {
    fn store(&self) -> Result<Store, String> {
        Store::open(self.root.join(".zcomposer/state.sqlite")).map_err(|error| error.to_string())
    }
}

#[tauri::command]
async fn snapshot(project: tauri::State<'_, Project>) -> Result<Value, String> {
    let snapshot = project
        .store()?
        .snapshot()
        .map_err(|error| error.to_string())?;
    let sessions: Vec<_> = snapshot.sessions.into_iter().map(|session| json!({
        "id": session.id, "agent": session.agent, "task": session.task, "state": session.state
    })).collect();
    Ok(json!({"cursor": snapshot.cursor, "sessions": sessions}))
}

#[tauri::command]
async fn answer(project: tauri::State<'_, Project>, id: i64) -> Result<String, String> {
    project
        .store()?
        .answer(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn live_answer(project: tauri::State<'_, Project>, id: i64) -> Result<Option<String>, String> {
    let draft = project.draft.lock().map_err(|error| error.to_string())?;
    Ok(draft
        .as_ref()
        .filter(|(session, _)| *session == id)
        .map(|(_, text)| text.clone()))
}

#[tauri::command]
async fn session_history(
    project: tauri::State<'_, Project>,
    id: i64,
) -> Result<Vec<String>, String> {
    project
        .store()?
        .session_history(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn studio_order(project: tauri::State<'_, Project>) -> Result<Vec<i64>, String> {
    project
        .store()?
        .studio_order()
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_studio_order(
    project: tauri::State<'_, Project>,
    ids: Vec<i64>,
) -> Result<(), String> {
    project
        .store()?
        .save_studio_order(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn run_turn(
    project: tauri::State<'_, Project>,
    window: Window,
    prompt: String,
) -> Result<Value, String> {
    if prompt.trim().is_empty() || prompt.len() > 16_384 {
        return Err("Informe uma tarefa de até 16 KiB.".into());
    }
    let cancel = project
        .lifecycle
        .lock()
        .map_err(|error| error.to_string())?
        .start_turn()?;
    let root = project.root.clone();
    let draft = Arc::clone(&project.draft);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut store =
            Store::open(root.join(".zcomposer/state.sqlite")).map_err(|error| error.to_string())?;
        zcomposer_adapters::run_codex_turn(&mut store, &root, &prompt, Some(&cancel), |id, text| {
            if let Ok(mut draft) = draft.lock() {
                *draft = Some((id, text.to_owned()));
            }
        })
        .map_err(|error| error.to_string())
    })
    .await;
    *project.draft.lock().map_err(|error| error.to_string())? = None;
    let close = project
        .lifecycle
        .lock()
        .map_err(|error| error.to_string())?
        .finish_turn();
    if close && let Err(error) = window.close() {
        project
            .lifecycle
            .lock()
            .map_err(|error| error.to_string())?
            .closing = false;
        return Err(format!("Não foi possível fechar a janela: {error}"));
    }
    let (id, state) = result.map_err(|error| error.to_string())??;
    Ok(json!({"id": id, "state": state.as_str()}))
}

#[tauri::command]
fn interrupt_turn(project: tauri::State<'_, Project>) -> Result<bool, String> {
    let lifecycle = project
        .lifecycle
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(lifecycle.interrupt_turn())
}

fn main() {
    let root = std::env::current_dir().expect("diretório do projeto");
    fs::create_dir_all(root.join(".zcomposer")).expect("diretório de dados local");
    tauri::Builder::default()
        .manage(Project {
            root,
            lifecycle: Mutex::new(WindowLifecycle::default()),
            draft: Arc::new(Mutex::new(None)),
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let project = window.state::<Project>();
                let mut lifecycle = project
                    .lifecycle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if lifecycle.request_close() {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            answer,
            live_answer,
            session_history,
            studio_order,
            save_studio_order,
            run_turn,
            interrupt_turn
        ])
        .run(tauri::generate_context!())
        .expect("iniciar janela do Zcomposer");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_signals_active_turn_and_blocks_new_work() {
        let mut lifecycle = WindowLifecycle::default();
        let cancel = lifecycle.start_turn().unwrap();
        assert!(lifecycle.request_close());
        assert!(cancel.load(Ordering::Acquire));
        assert!(lifecycle.finish_turn());
        assert!(lifecycle.start_turn().is_err());
        assert!(!lifecycle.request_close());
    }
}
