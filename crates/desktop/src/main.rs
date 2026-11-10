use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};
use zcomposer_core::Store;

struct Project {
    root: PathBuf,
    running: AtomicBool,
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
async fn run_turn(project: tauri::State<'_, Project>, prompt: String) -> Result<i64, String> {
    if prompt.trim().is_empty() || prompt.len() > 16_384 {
        return Err("Informe uma tarefa de até 16 KiB.".into());
    }
    if project.running.swap(true, Ordering::AcqRel) {
        return Err("Já existe um turno em execução nesta janela.".into());
    }
    let root = project.root.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut store =
            Store::open(root.join(".zcomposer/state.sqlite")).map_err(|error| error.to_string())?;
        zcomposer_adapters::run_codex(&mut store, &root, &prompt).map_err(|error| error.to_string())
    })
    .await;
    project.running.store(false, Ordering::Release);
    result.map_err(|error| error.to_string())?
}

fn main() {
    let root = std::env::current_dir().expect("diretório do projeto");
    fs::create_dir_all(root.join(".zcomposer")).expect("diretório de dados local");
    tauri::Builder::default()
        .manage(Project {
            root,
            running: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![snapshot, answer, run_turn])
        .run(tauri::generate_context!())
        .expect("iniciar janela do Zcomposer");
}
