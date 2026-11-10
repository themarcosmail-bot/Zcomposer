use std::{env, error::Error, fs, path::Path};

use zcomposer_core::{State, Store};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let command = match args.as_slice() {
        [command, ..] => command.as_str(),
        [] => "help",
    };
    if matches!(command, "help" | "--help" | "-h") {
        println!("zcomposer demo | codex <tarefa> | answer <id> | sessions | events [cursor]");
        return Ok(());
    }

    let directory = Path::new(".zcomposer");
    fs::create_dir_all(directory)?;
    let mut store = Store::open(directory.join("state.sqlite"))?;
    match args.as_slice() {
        [command, prompt] if command == "codex" && !prompt.trim().is_empty() => {
            let id = zcomposer_adapters::run_codex(&mut store, &env::current_dir()?, prompt)?;
            println!("Sessão {id}\n{}", store.answer(id)?);
        }
        [command, id] if command == "answer" => {
            println!("{}", store.answer(id.parse()?)?);
        }
        [command] if command == "demo" => {
            let id = store.create("demo", "Validar histórico de uma sessão simulada")?;
            if !store.transition(id, State::Running)? || !store.transition(id, State::Completed)? {
                return Err("a sessão de demonstração mudou de estado durante a execução".into());
            }
            println!("Sessão simulada {id} concluída.");
        }
        [command] if command == "sessions" => {
            let snapshot = store.snapshot()?;
            println!("Cursor {}", snapshot.cursor);
            for session in snapshot.sessions {
                println!(
                    "{}\t{}\t{}\t{}",
                    session.id, session.agent, session.state, session.task
                );
            }
        }
        [command, rest @ ..] if command == "events" && rest.len() <= 1 => {
            let cursor = rest
                .first()
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(0);
            for event in store.events_after(cursor)? {
                println!("{}\t{}\t{}", event.cursor, event.session_id, event.state);
            }
        }
        _ => {
            return Err(
                "uso: zcomposer demo | codex <tarefa> | answer <id> | sessions | events [cursor]"
                    .into(),
            );
        }
    }
    Ok(())
}
