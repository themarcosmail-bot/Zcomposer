use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
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
    let id = store.create("codex", prompt)?;
    if !store.transition(id, State::Running)? {
        return Err("não foi possível iniciar a sessão".into());
    }
    let outcome = (|| {
        let mut server = Server::start(cwd)?;
        server.run(cwd, prompt)
    })();
    match outcome {
        Ok((thread_id, answer)) => {
            if !store.finish_turn(id, &thread_id, &answer)? {
                return Err("a sessão mudou antes de salvar a resposta".into());
            }
            Ok(id)
        }
        Err(error) => {
            store.transition(id, State::Failed)?;
            Err(format!("sessão {id}: {error}").into())
        }
    }
}

struct Server {
    child: Child,
    input: ChildStdin,
    messages: Receiver<std::io::Result<String>>,
    deadline: Instant,
}

impl Server {
    fn start(cwd: &Path) -> Result<Self> {
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
        })
    }

    fn send(&mut self, message: Value) -> Result<()> {
        writeln!(self.input, "{message}")?;
        self.input.flush()?;
        Ok(())
    }

    fn receive(&mut self) -> Result<Value> {
        loop {
            let timeout = self.deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err("o prazo do turno do Codex expirou".into());
            }
            let message: Value = serde_json::from_str(&self.messages.recv_timeout(timeout)??)?;
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
        loop {
            let message = self.receive()?;
            if message["id"].as_u64() == Some(3)
                && let Some(error) = message.get("error")
            {
                return Err(format!("turn/start: {}", error["message"]).into());
            }
            let params = &message["params"];
            if params["threadId"].as_str() != Some(thread_id) {
                continue;
            }
            match message["method"].as_str() {
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

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
