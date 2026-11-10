use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use zcomposer_core::{State, Store};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn codex_executable() -> Result<PathBuf> {
    let path = env::var_os("PATH").ok_or("PATH indisponível")?;
    let filename = if cfg!(windows) { "codex.exe" } else { "codex" };
    env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(filename))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{filename} não encontrado no PATH").into())
}

pub fn run_codex(store: &mut Store, cwd: &Path, prompt: &str) -> Result<i64> {
    let (id, _) = run_codex_turn(store, cwd, prompt, None)?;
    Ok(id)
}

pub fn run_codex_turn(
    store: &mut Store,
    cwd: &Path,
    prompt: &str,
    cancel: Option<&AtomicBool>,
) -> Result<(i64, State)> {
    let id = store.create("codex", prompt)?;
    if !store.transition(id, State::Running)? {
        return Err("não foi possível iniciar a sessão".into());
    }
    let outcome = (|| {
        let mut server = Server::start(cwd, cancel)?;
        server.run(cwd, prompt)
    })();
    match outcome {
        Ok((thread_id, answer)) => {
            if !store.finish_turn(id, &thread_id, &answer)? {
                return Err("a sessão mudou antes de salvar a resposta".into());
            }
            Ok((id, State::Completed))
        }
        Err(error) if error.is::<Interrupted>() => {
            if !store.transition(id, State::Interrupted)? {
                return Err("a sessão mudou antes de registrar a interrupção".into());
            }
            Ok((id, State::Interrupted))
        }
        Err(error) => {
            store.transition(id, State::Failed)?;
            Err(format!("sessão {id}: {error}").into())
        }
    }
}

#[derive(Debug)]
struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("turno interrompido")
    }
}

impl std::error::Error for Interrupted {}

struct Server<'a> {
    child: Child,
    input: ChildStdin,
    messages: Receiver<std::io::Result<String>>,
    deadline: Instant,
    cancel: Option<&'a AtomicBool>,
}

impl<'a> Server<'a> {
    fn start(cwd: &Path, cancel: Option<&'a AtomicBool>) -> Result<Self> {
        if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(Interrupted.into());
        }
        let mut child = Command::new(codex_executable()?)
            .args(["app-server", "--stdio", "-c", "mcp_servers={}"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take().expect("stdin is piped");
        let output = child.stdout.take().expect("stdout is piped");
        let (sender, messages) = mpsc::sync_channel(32);
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            messages,
            deadline: Instant::now() + Duration::from_secs(120),
            cancel,
        })
    }

    fn send(&mut self, message: Value) -> Result<()> {
        writeln!(self.input, "{message}")?;
        self.input.flush()?;
        Ok(())
    }

    fn receive(&mut self) -> Result<Value> {
        loop {
            if self.cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(Interrupted.into());
            }
            let timeout = self.deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err("o prazo do turno do Codex expirou".into());
            }
            let interval = if self.cancel.is_some() {
                timeout.min(Duration::from_millis(100))
            } else {
                timeout
            };
            let line = match self.messages.recv_timeout(interval) {
                Ok(line) => line?,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(error) => return Err(error.into()),
            };
            let message: Value = serde_json::from_str(&line)?;
            if let (Some(id), Some(_)) = (message.get("id"), message.get("method")) {
                self.send(json!({
                    "id": id,
                    "error": {"code": -32601, "message": "Interactive requests are disabled"}
                }))?;
                continue;
            }
            return Ok(message);
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        self.send(json!({"id": id, "method": method, "params": params}))?;
        loop {
            let message = self.receive()?;
            if message["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!("{method}: {}", error["message"]).into());
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "resposta RPC sem resultado".into());
        }
    }

    fn run(&mut self, cwd: &Path, prompt: &str) -> Result<(String, String)> {
        self.request(
            1,
            "initialize",
            json!({"clientInfo": {
                "name": "zcomposer", "title": "Zcomposer", "version": env!("CARGO_PKG_VERSION")
            }}),
        )?;
        self.send(json!({"method": "initialized"}))?;
        let response = self.request(
            2,
            "thread/start",
            json!({
                "cwd": cwd, "sandbox": "read-only", "approvalPolicy": "never", "ephemeral": true
            }),
        )?;
        let thread_id = response["thread"]["id"].as_str().ok_or("thread sem ID")?;
        // Observe notifications even when they precede the turn/start response.
        self.send(json!({"id": 3, "method": "turn/start", "params": {
            "threadId": thread_id, "input": [{"type": "text", "text": prompt}],
            "approvalPolicy": "never", "sandboxPolicy": {"type": "readOnly", "networkAccess": false}
        }}))?;
        let mut answer = String::new();
        let mut turn_id = None;
        let mut interrupting = false;
        loop {
            let message = match self.receive() {
                Err(error) if error.is::<Interrupted>() => {
                    let Some(id) = &turn_id else {
                        return Err(error);
                    };
                    self.cancel = None;
                    self.deadline = Instant::now() + Duration::from_secs(2);
                    if self
                        .send(json!({
                            "id": 4, "method": "turn/interrupt",
                            "params": {"threadId": thread_id, "turnId": id}
                        }))
                        .is_err()
                    {
                        return Err(error);
                    }
                    interrupting = true;
                    continue;
                }
                Err(_) if interrupting => return Err(Interrupted.into()),
                result => result?,
            };
            if message["id"].as_u64() == Some(3) {
                if let Some(error) = message.get("error") {
                    return Err(format!("turn/start: {}", error["message"]).into());
                }
                turn_id = message["result"]["turn"]["id"].as_str().map(str::to_owned);
            }
            let params = &message["params"];
            if params["threadId"].as_str() != Some(thread_id) {
                continue;
            }
            match message["method"].as_str() {
                Some("turn/started") => {
                    if let Some(id) = params["turn"]["id"].as_str() {
                        turn_id = Some(id.to_owned());
                    }
                }
                Some("item/completed") if params["item"]["type"] == "agentMessage" => {
                    if let Some(text) = params["item"]["text"].as_str() {
                        if !answer.is_empty() {
                            answer.push_str("\n\n");
                        }
                        answer.push_str(text);
                    }
                }
                Some("turn/completed") if params["turn"]["status"] == "completed" => {
                    return Ok((thread_id.to_owned(), answer));
                }
                Some("turn/completed") if params["turn"]["status"] == "interrupted" => {
                    return Err(Interrupted.into());
                }
                Some("turn/completed") => {
                    return Err(
                        format!("turno encerrado com estado {}", params["turn"]["status"]).into(),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Drop for Server<'_> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_launch_is_recorded_as_interrupted() -> Result<()> {
        let mut store = Store::open(":memory:")?;
        let cancel = AtomicBool::new(true);
        let (id, state) = run_codex_turn(
            &mut store,
            Path::new("unused"),
            "Tarefa de teste",
            Some(&cancel),
        )?;
        assert_eq!(state, State::Interrupted);
        assert_eq!(
            store.session_history(id)?,
            ["queued", "running", "interrupted"]
        );
        assert!(store.answer(id).is_err());
        Ok(())
    }
}
