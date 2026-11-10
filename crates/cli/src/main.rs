use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::Path,
};

use zcomposer_core::{ReviewReport, State, Store};

const USAGE: &str = "zcomposer demo | codex <tarefa> | review <commit> | review-report <id> | answer <id> | sessions | events [cursor]";

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let command = match args.as_slice() {
        [command, ..] => command.as_str(),
        [] => "help",
    };
    if matches!(command, "help" | "--help" | "-h") {
        println!("{USAGE}");
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
        [command, revision] if command == "review" && !revision.trim().is_empty() => {
            let id = zcomposer_adapters::review_commit(&mut store, &env::current_dir()?, revision)?;
            println!("Sessão {id}\n{}", store.answer(id)?);
        }
        [command, id] if command == "review-report" => {
            let id = id.parse()?;
            let report = store
                .review_report(id)?
                .ok_or("Informe o ID de uma revisão concluída com parecer salvo.")?;
            write_review_report(io::stdout().lock(), id, &report)?;
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
        _ => return Err(format!("uso: {USAGE}").into()),
    }
    Ok(())
}

fn write_review_report(mut output: impl Write, id: i64, report: &ReviewReport) -> io::Result<()> {
    let decision = match report.approved {
        Some(true) => "Aprovada",
        Some(false) => "Ajustes solicitados",
        None => "Pendente",
    };
    writeln!(
        output,
        "# Revisão do Codex\n\n- Sessão `#{id}`\n- Decisão humana — {decision}"
    )?;
    if let Some(commit) = report
        .task
        .strip_prefix("Revisar commit ")
        .filter(|commit| {
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        writeln!(output, "- Commit `{commit}`")?;
    }
    writeln!(output, "\n## Parecer\n")?;
    for line in report.answer.lines() {
        writeln!(output, "    {line}")?;
    }
    Ok(())
}
